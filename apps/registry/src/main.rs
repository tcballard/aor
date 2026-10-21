use aor_db::{Pool, PoolOptions};
use aor_session::Config;
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if !["serve", "migrate", "routes", "work", "jobs"].contains(&command.as_str()) {
        return Err("usage: aor-registry [serve|migrate|routes|work [--once]]".into());
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
            "list" => 0,
            "inspect" | "retry" | "park" => 1,
            _ => return Err("unknown jobs command".into()),
        };
        if remaining.len() != expected {
            return Err("unexpected jobs arguments".into());
        }
        let at = aor_db::Utc::now().timestamp();
        let value=match action.as_str() {
            "list"=>serde_json::Value::Array(aor_jobs::list(&pool,"registry",100).await?.into_iter().map(|r|serde_json::json!({"id":r.id,"job_type":r.job_type,"state":r.state,"attempt":r.attempt,"max_attempts":r.max_attempts,"run_at":r.run_at,"last_error":r.last_error})).collect()),
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
        if once {
            aor_registry::manifests::work_once(&pool, aor_db::Utc::now().timestamp()).await?;
            return Ok(());
        }
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let stop = async {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
        };
        tokio::pin!(stop);
        loop {
            if let Err(error) =
                aor_registry::manifests::work_once(&pool, aor_db::Utc::now().timestamp()).await
            {
                eprintln!("{error}");
            }
            tokio::select! { _=&mut stop=>break, _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{} }
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
    let router = aor_registry::router(
        pool,
        Config {
            secure_cookie: origin.starts_with("https://"),
            public_origin: origin,
            production,
            strict_cookie: false,
            idle_seconds: 1800,
            absolute_seconds: 86400 * 7,
            registration_enabled: mail.is_some(),
        },
        mail,
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
