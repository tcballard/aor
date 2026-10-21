use aor_db::{Pool, PoolOptions};
use aor_session::Config;
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if ["--help", "-h"].contains(&command.as_str()) {
        println!("usage: aor-registry [serve|migrate|routes|work [--once]|jobs|mail-keygen FILE]");
        return Ok(());
    }
    if !["serve", "migrate", "routes", "work", "jobs", "mail-keygen"].contains(&command.as_str()) {
        return Err("usage: aor-registry [serve|migrate|routes|work [--once]]".into());
    }
    if command == "mail-keygen" {
        let path = args.next().ok_or("usage: aor-registry mail-keygen FILE")?;
        if args.next().is_some() {
            return Err("unexpected keygen argument".into());
        }
        aor_mail::generate_key(std::path::Path::new(&path))?;
        println!("created private mail signing key");
        return Ok(());
    }
    let pool = if let Ok(url) = std::env::var("AOR_DATABASE_URL") {
        Pool::postgres(&url, PoolOptions::default())?
    } else {
        Pool::sqlite(
            std::env::var("AOR_SQLITE").unwrap_or_else(|_| "registry.sqlite".into()),
            PoolOptions::default(),
        )?
    };
    if command == "migrate" {
        println!("{}", aor_registry::migrate(&pool).await?);
        return Ok(());
    }
    if command == "jobs" {
        let action = args
            .next()
            .ok_or("usage: aor-registry jobs <list|inspect|retry|park> [id]")?;
        let remaining: Vec<_> = args.collect();
        let expected = match action.as_str() {
            "list" if remaining.is_empty() => 0,
            "list"
                if remaining.len() == 2
                    && remaining[0] == "--queue"
                    && ["mail", "registry"].contains(&remaining[1].as_str()) =>
            {
                2
            }
            "inspect" | "retry" | "park" => 1,
            _ => return Err("unknown jobs command".into()),
        };
        if remaining.len() != expected {
            return Err("unexpected jobs arguments".into());
        }
        let at = aor_db::Utc::now().timestamp();
        let queue = if remaining.len() == 2 {
            remaining[1].as_str()
        } else {
            "registry"
        };
        let value=match action.as_str() {
            "list"=>serde_json::Value::Array(aor_jobs::list(&pool,queue,100).await?.into_iter().map(|r|serde_json::json!({"id":r.id,"job_type":r.job_type,"state":r.state,"attempt":r.attempt,"max_attempts":r.max_attempts,"run_at":r.run_at,"last_error":r.last_error})).collect()),
            "inspect"=> {
                let id=&remaining[0];
                let r=aor_jobs::inspect(&pool,id).await?.ok_or("job not found")?;
                serde_json::json!({"id":r.id,"job_type":r.job_type,"payload_version":r.payload_version,"queue":r.queue,"state":r.state,"attempt":r.attempt,"max_attempts":r.max_attempts,"run_at":r.run_at,"lease_expires_at":r.lease_expires_at,"lease_generation":r.lease_generation,"created_at":r.created_at,"updated_at":r.updated_at,"last_error":r.last_error})
            }
            "retry"|"park"=> {
                let id=&remaining[0];
                if action=="retry" {aor_jobs::requeue(&pool,id,at).await?;} else {aor_jobs::park(&pool,id,at).await?;}
                serde_json::json!({"id":id,"action":action,"ok":true})
            }
            _=>return Err("unknown jobs command".into()),
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    if command == "work" {
        let once = match args.next().as_deref() {
            None => false,
            Some("--once") => true,
            _ => return Err("usage: aor-registry work [--once]".into()),
        };
        if args.next().is_some() {
            return Err("unexpected worker argument".into());
        }
        let mail = match (
            std::env::var_os("AOR_MAIL_KEY_FILE"),
            std::env::var_os("AOR_RESEND_KEY_FILE"),
        ) {
            (None, None) => None,
            (Some(key), Some(api)) => Some((
                aor_mail::Key::load(std::path::Path::new(&key))?,
                aor_mail::Resend::from_key_file(std::path::Path::new(&api))?,
            )),
            _ => {
                return Err(
                    "mail worker requires both AOR_MAIL_KEY_FILE and AOR_RESEND_KEY_FILE".into(),
                );
            }
        };
        let runtime = aor_registry::worker::Runtime::new(pool.clone(), mail);
        if once {
            if let Some(task) = runtime.claim(false).await? {
                runtime.process(task).await?;
            }
            return Ok(());
        }
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let stop = async {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
        };
        tokio::pin!(stop);
        let mut mail_first = false;
        loop {
            let task =
                tokio::select! {biased;_=&mut stop=>break,result=runtime.claim(mail_first)=>result};
            mail_first = !mail_first;
            match task {
                Err(error) => eprintln!("{error}"),
                Ok(Some(task)) => {
                    let work = runtime.process(task);
                    tokio::pin!(work);
                    tokio::select! {
                        result=&mut work=>if let Err(error)=result {eprintln!("{error}");},
                        _=&mut stop=>{let _=tokio::time::timeout(std::time::Duration::from_secs(30),&mut work).await;break;}
                    }
                }
                Ok(None) => {}
            }
            tokio::select! {_=&mut stop=>break,_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
        }
        return Ok(());
    }
    let production = std::env::var("AOR_ENV").as_deref() == Ok("production");
    let origin = std::env::var("AOR_PUBLIC_ORIGIN").or_else(|e| {
        if production {
            Err(e)
        } else {
            Ok("http://localhost:3001".into())
        }
    })?;
    let mail = std::env::var_os("AOR_DEV_MAIL_SPOOL").map(std::path::PathBuf::from);
    let outbox = match (
        std::env::var_os("AOR_MAIL_KEY_FILE"),
        std::env::var("AOR_MAIL_FROM").ok(),
    ) {
        (None, None) => None,
        (Some(path), Some(sender)) => Some(aor_mail::Outbox::new(
            aor_mail::Key::load(std::path::Path::new(&path))?,
            sender,
        )?),
        _ => return Err("queued mail requires both AOR_MAIL_KEY_FILE and AOR_MAIL_FROM".into()),
    };
    let enabled = mail.is_some() || outbox.is_some();
    let router = aor_registry::router_with_outbox(
        pool,
        Config {
            secure_cookie: origin.starts_with("https://"),
            public_origin: origin,
            production,
            strict_cookie: false,
            idle_seconds: 1800,
            absolute_seconds: 86400 * 7,
            registration_enabled: enabled,
        },
        mail,
        outbox,
    )
    .await?;
    if command == "routes" {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"schema_version":1,"routes":router.routes(),"middleware":aor_router::AUTHENTICATED_MIDDLEWARE})
            )?
        );
        return Ok(());
    }
    let listener = if let Ok(path) = std::env::var("AOR_SOCKET") {
        aor_http::Listener::unix(std::path::Path::new(&path))?
    } else {
        aor_http::Listener::tcp("127.0.0.1:3001".parse()?).await?
    };
    let router = Arc::new(router);
    aor_http::serve(
        listener,
        aor_http::Limits::default(),
        move |request| {
            let router = router.clone();
            async move { router.handle(request).await }
        },
        async {
            let _ = tokio::signal::ctrl_c().await;
        },
    )
    .await?;
    Ok(())
}
