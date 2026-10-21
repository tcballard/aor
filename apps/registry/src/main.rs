use aor_db::{Pool, PoolOptions};
use aor_session::Config;
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if !["serve", "migrate", "routes"].contains(&command.as_str()) {
        return Err("usage: aor-registry [serve|migrate|routes]".into());
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
