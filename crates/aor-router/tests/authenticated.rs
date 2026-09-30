use aor_db::{Pool, PoolOptions, Uuid};
use aor_http::{Body, Request, Response};
use aor_router::{Route, Router};
use aor_session::{Auth, Config};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[tokio::test]
async fn csrf_precedes_hooks_and_handlers_and_form_body_is_replayed() {
    let file = std::env::temp_dir().join(format!("aor-router-auth-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    let auth = Auth::new(
        pool,
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
    .unwrap();
    auth.migrate().await.unwrap();
    let d = auth
        .register("user@example.test", "long test password".into())
        .await
        .unwrap()
        .unwrap();
    auth.verify_email(d.token.expose()).await.unwrap();
    let login = auth
        .login("user@example.test", "long test password".into(), None)
        .await
        .unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let router = Router::new(vec![Route::protected(
        "POST",
        "/mutate",
        "mutate",
        "owner",
        move |ctx| {
            let h = h.clone();
            async move {
                assert!(ctx.principal().is_some());
                let body = ctx.request.body.collect().await.unwrap();
                assert!(std::str::from_utf8(&body).unwrap().contains("name=example"));
                h.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200, vec![]))
            }
        },
    )])
    .unwrap()
    .with_auth(auth.clone())
    .before_route({
        let hits = hits.clone();
        move |_, _| {
            hits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });
    for (body, status) in [
        ("name=example".to_owned(), 403),
        (format!("name=example&_csrf={}", login.csrf.expose()), 200),
        (
            format!(
                "name=example&_csrf={}&_csrf={}",
                login.csrf.expose(),
                login.csrf.expose()
            ),
            400,
        ),
    ] {
        let headers = vec![
            (
                "Content-Type".into(),
                b"application/x-www-form-urlencoded".to_vec(),
            ),
            ("Origin".into(), b"http://localhost:3000".to_vec()),
            (
                "Cookie".into(),
                format!(
                    "aor_dev_session={}; aor_dev_csrf={}",
                    login.session.expose(),
                    login.csrf.expose()
                )
                .into_bytes(),
            ),
        ];
        let response = router
            .handle(Request {
                method: "POST".into(),
                target: "/mutate".into(),
                headers,
                body: Body::from_bytes(body.into_bytes(), 4096).unwrap(),
            })
            .await;
        assert_eq!(response.status, status);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    std::fs::remove_file(file).unwrap();
}
