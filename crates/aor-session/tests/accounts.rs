use aor_db::{Pool, PoolOptions, Uuid};
use aor_session::{Auth, Config, Error, Login};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
fn config() -> Config {
    Config {
        public_origin: "http://localhost:3000".into(),
        production: false,
        secure_cookie: false,
        strict_cookie: false,
        idle_seconds: 60,
        absolute_seconds: 120,
        registration_enabled: true,
    }
}
fn headers(login: &Login, csrf: bool) -> Vec<(String, Vec<u8>)> {
    let mut h = vec![(
        "Cookie".into(),
        format!(
            "aor_dev_session={}; aor_dev_csrf={}",
            login.session.expose(),
            login.csrf.expose()
        )
        .into_bytes(),
    )];
    if csrf {
        h.push(("Origin".into(), b"http://localhost:3000".to_vec()));
        h.push((
            "X-CSRF-Token".into(),
            login.csrf.expose().as_bytes().to_vec(),
        ));
    }
    h
}
async fn suite(pool: Pool) {
    let clock = Arc::new(AtomicI64::new(1000));
    let c = clock.clone();
    let auth = Auth::new(pool.clone(), config())
        .await
        .unwrap()
        .with_clock(move || c.load(Ordering::SeqCst));
    auth.migrate().await.unwrap();
    let password = "a long test password";
    let delivery = auth
        .register("a@example.test", password.into())
        .await
        .unwrap()
        .unwrap();
    assert!(
        auth.login("a@example.test", password.into(), None)
            .await
            .is_err()
    );
    auth.verify_email(delivery.token.expose()).await.unwrap();
    assert!(auth.verify_email(delivery.token.expose()).await.is_err());
    assert!(
        auth.register("A@example.test", password.into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        auth.login("missing@example.test", password.into(), None)
            .await
            .is_err()
    );
    assert!(
        auth.login("a@example.test", "wrong password".into(), None)
            .await
            .is_err()
    );
    let first = auth
        .login("a@example.test", password.into(), None)
        .await
        .unwrap();
    let login = auth
        .login(
            "a@example.test",
            password.into(),
            Some(first.session.expose()),
        )
        .await
        .unwrap();
    assert_ne!(first.session.expose(), login.session.expose());
    assert!(
        auth.authenticate(&headers(&first, false), "GET")
            .await
            .is_err()
    );
    assert!(matches!(
        auth.authenticate(&headers(&login, false), "POST").await,
        Err(Error::Csrf)
    ));
    let good = headers(&login, true);
    let identity = auth.authenticate(&good, "POST").await.unwrap().unwrap();
    let mut bad = good.clone();
    bad.push((
        "X-CSRF-Token".into(),
        login.csrf.expose().as_bytes().to_vec(),
    ));
    assert!(auth.authenticate(&bad, "POST").await.is_err());
    let mut bad = good.clone();
    bad[1].1 = b"https://evil.test".to_vec();
    assert!(auth.authenticate(&bad, "POST").await.is_err());
    let token = auth
        .create_api_token(identity.principal(), vec!["plugins:read".into()], 100)
        .await
        .unwrap();
    let bearer = vec![(
        "Authorization".into(),
        format!("Bearer {}", token.expose()).into_bytes(),
    )];
    let api = auth.authenticate(&bearer, "POST").await.unwrap().unwrap();
    assert!(api.principal().permits("plugins", "read"));
    assert!(!api.principal().permits("plugins", "delete"));
    assert!(auth.revoke_all(api.principal()).await.is_err());
    let mut mixed = good.clone();
    mixed.extend(bearer.clone());
    assert!(auth.authenticate(&mixed, "GET").await.is_err());
    auth.revoke_api_token(identity.principal(), token.expose())
        .await
        .unwrap();
    assert!(auth.authenticate(&bearer, "GET").await.is_err());
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        assert!(auth.validate_in(&mut tx, api.principal()).await.is_err());
        tx.rollback().await.unwrap();
    }
    auth.logout(identity.session().unwrap()).await.unwrap();
    assert!(auth.authenticate(&good, "GET").await.is_err());
    let login = auth
        .login("a@example.test", password.into(), None)
        .await
        .unwrap();
    let current = auth
        .authenticate(&headers(&login, false), "GET")
        .await
        .unwrap()
        .unwrap();
    let token = auth
        .create_api_token(current.principal(), vec!["plugins:read".into()], 100)
        .await
        .unwrap();
    assert!(
        auth.request_reset("unknown@example.test")
            .await
            .unwrap()
            .is_none()
    );
    let reset = auth.request_reset("a@example.test").await.unwrap().unwrap();
    auth.reset_password(reset.token.expose(), "replacement password".into())
        .await
        .unwrap();
    assert!(
        auth.reset_password(reset.token.expose(), "another password".into())
            .await
            .is_err()
    );
    assert!(
        auth.authenticate(&headers(&login, false), "GET")
            .await
            .is_err()
    );
    assert!(
        auth.authenticate(
            &[(
                "Authorization".into(),
                format!("Bearer {}", token.expose()).into_bytes()
            )],
            "GET"
        )
        .await
        .is_err()
    );
    assert!(
        auth.login("a@example.test", password.into(), None)
            .await
            .is_err()
    );
    let login = auth
        .login("a@example.test", "replacement password".into(), None)
        .await
        .unwrap();
    clock.store(1060, Ordering::SeqCst);
    assert!(
        auth.authenticate(&headers(&login, false), "GET")
            .await
            .is_err()
    );
    let login = auth
        .login("a@example.test", "replacement password".into(), None)
        .await
        .unwrap();
    for t in [1100, 1140] {
        clock.store(t, Ordering::SeqCst);
        assert!(
            auth.authenticate(&headers(&login, false), "GET")
                .await
                .unwrap()
                .is_some()
        );
    }
    clock.store(1180, Ordering::SeqCst);
    assert!(
        auth.authenticate(&headers(&login, false), "GET")
            .await
            .is_err()
    );
    let login = auth
        .login("a@example.test", "replacement password".into(), None)
        .await
        .unwrap();
    let current = auth
        .authenticate(&headers(&login, false), "GET")
        .await
        .unwrap()
        .unwrap();
    auth.revoke_all(current.principal()).await.unwrap();
    assert!(
        auth.authenticate(&headers(&login, false), "GET")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn sqlite_account_denials() {
    let file = std::env::temp_dir().join(format!("aor-auth-{}.sqlite", Uuid::new_v4()));
    suite(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_account_denials() {
    let url = std::env::var("AOR_TEST_DATABASE_URL").unwrap();
    let pool = Pool::postgres(&url, PoolOptions::default()).unwrap();
    suite(pool).await;
}
#[test]
fn production_cookie_configuration() {
    let mut c = config();
    c.production = true;
    assert!(c.validate().is_err());
    c.public_origin = "https://example.test".into();
    assert!(c.validate().is_err());
    c.secure_cookie = true;
    assert!(c.validate().is_ok());
    for origin in [
        "https://example.test/",
        "https://user@example.test",
        "https://example.test?x",
        "https://example.test\n",
    ] {
        c.public_origin = origin.into();
        assert!(c.validate().is_err());
    }
}
