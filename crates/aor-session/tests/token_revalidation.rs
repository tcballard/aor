use aor_db::{Pool, PoolOptions, Uuid};
use aor_session::{Auth, Config, Error};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
async fn suite(pool: Pool) {
    let clock = Arc::new(AtomicI64::new(1000));
    let c = clock.clone();
    let auth = Auth::new(
        pool.clone(),
        Config {
            public_origin: "http://localhost:3000".into(),
            production: false,
            secure_cookie: false,
            strict_cookie: false,
            idle_seconds: 60,
            absolute_seconds: 120,
            registration_enabled: true,
        },
    )
    .await
    .unwrap()
    .with_clock(move || c.load(Ordering::SeqCst));
    auth.migrate().await.unwrap();
    let delivery = auth
        .register(
            "token-revalidation@example.test",
            "a strong test password".into(),
        )
        .await
        .unwrap()
        .unwrap();
    auth.verify_email(delivery.token.expose()).await.unwrap();
    for expired in [false, true] {
        let login = auth
            .login(
                "token-revalidation@example.test",
                "a strong test password".into(),
                None,
            )
            .await
            .unwrap();
        let identity = auth
            .authenticate(
                &[(
                    "Cookie".into(),
                    format!("aor_dev_session={}", login.session.expose()).into_bytes(),
                )],
                "GET",
            )
            .await
            .unwrap()
            .unwrap();
        if expired {
            clock.store(1060, Ordering::SeqCst);
        } else {
            auth.logout(identity.session().unwrap()).await.unwrap();
        }
        assert!(matches!(
            auth.create_api_token(identity.principal(), vec!["plugins:read".into()], 60)
                .await,
            Err(Error::Unauthenticated)
        ));
    }
    pool.close();
}
#[tokio::test]
async fn sqlite_logged_out_and_expired_principals_cannot_mint_tokens() {
    let file =
        std::env::temp_dir().join(format!("aor-token-revalidation-{}.sqlite", Uuid::new_v4()));
    suite(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_logged_out_and_expired_principals_cannot_mint_tokens() {
    suite(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
    )
    .await;
}
