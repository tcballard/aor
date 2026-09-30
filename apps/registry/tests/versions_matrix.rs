use aor_db::{Pool, PoolOptions, Uuid};
use aor_http::{Body, Request};
use aor_router::Router;
use aor_session::{Auth, Config, Login};
use serde_json::{Value, json};
use std::sync::Arc;
fn config() -> Config {
    Config {
        public_origin: "http://localhost:3001".into(),
        production: false,
        secure_cookie: false,
        strict_cookie: false,
        idle_seconds: 1800,
        absolute_seconds: 86400,
        registration_enabled: true,
    }
}
async fn account(auth: &Auth, email: &str) -> Login {
    let d = auth
        .register(email, "a strong test password".into())
        .await
        .unwrap()
        .unwrap();
    auth.verify_email(d.token.expose()).await.unwrap();
    auth.login(email, "a strong test password".into(), None)
        .await
        .unwrap()
}
async fn request(
    router: &Arc<Router>,
    method: &str,
    path: &str,
    login: Option<&Login>,
    csrf: bool,
    input: Value,
) -> (u16, Value) {
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut headers: Vec<(String, Vec<u8>)> =
        vec![("Content-Type".into(), b"application/json".to_vec())];
    if csrf {
        headers.push(("Origin".into(), b"http://localhost:3001".to_vec()));
    }
    if let Some(login) = login {
        headers.push((
            "Cookie".into(),
            format!(
                "aor_dev_session={}; aor_dev_csrf={}",
                login.session.expose(),
                login.csrf.expose()
            )
            .into_bytes(),
        ));
        if csrf {
            headers.push((
                "X-CSRF-Token".into(),
                login.csrf.expose().as_bytes().to_vec(),
            ));
        }
    }
    let mut wire=format!("{method} {path} HTTP/1.1\r\nHost: localhost:3001\r\nConnection: close\r\nContent-Length: {}\r\n",bytes.len()).into_bytes();
    for (name, value) in headers {
        wire.extend_from_slice(name.as_bytes());
        wire.extend_from_slice(b": ");
        wire.extend_from_slice(&value);
        wire.extend_from_slice(b"\r\n");
    }
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(&bytes);
    let (status, body) = aor_testkit::exchange(router.clone(), wire).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn matrix(pool: Pool) {
    aor_registry::migrate(&pool).await.unwrap();
    let auth = Auth::new(pool.clone(), config()).await.unwrap();
    let alice = account(&auth, "alice@example.test").await;
    let bob = account(&auth, "bob@example.test").await;
    let spool = std::env::temp_dir().join(format!("aor-mail-{}", Uuid::new_v4()));
    let router = Arc::new(
        aor_registry::router(pool.clone(), config(), Some(spool.clone()))
            .await
            .unwrap(),
    );
    // Account HTTP responses conceal existence; login also rejects cross-origin POSTs.
    let signup = json!({"email":"new@example.test","password":"another strong password"});
    let fresh = request(
        &router,
        "POST",
        "/accounts/register",
        None,
        true,
        signup.clone(),
    )
    .await;
    let existing = request(&router, "POST", "/accounts/register", None, true, signup).await;
    assert_eq!(fresh.0, 202);
    assert_eq!(fresh.1, existing.1);
    let known = request(
        &router,
        "POST",
        "/accounts/request-reset",
        None,
        true,
        json!({"email":"alice@example.test"}),
    )
    .await;
    let unknown = request(
        &router,
        "POST",
        "/accounts/request-reset",
        None,
        true,
        json!({"email":"missing@example.test"}),
    )
    .await;
    assert_eq!(known.0, 202);
    assert_eq!(known.1, unknown.1);
    assert!(known.1.get("token").is_none());
    let credentials = json!({"email":"alice@example.test","password":"a strong test password"});
    assert_eq!(
        request(
            &router,
            "POST",
            "/accounts/login",
            None,
            false,
            credentials.clone()
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(&router, "POST", "/accounts/login", None, true, credentials)
            .await
            .0,
        200
    );
    let wrong = request(
        &router,
        "POST",
        "/accounts/login",
        None,
        true,
        json!({"email":"alice@example.test","password":"incorrect password"}),
    )
    .await;
    let missing = request(
        &router,
        "POST",
        "/accounts/login",
        None,
        true,
        json!({"email":"nobody@example.test","password":"incorrect password"}),
    )
    .await;
    assert_eq!(wrong.0, 401);
    assert_eq!(missing.0, 401);
    assert_eq!(wrong.1["error"]["code"], missing.1["error"]["code"]);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&spool).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in std::fs::read_dir(&spool).unwrap() {
        assert_eq!(
            file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(&spool).unwrap();
    let (status, plugin) = request(
        &router,
        "POST",
        "/plugins",
        Some(&alice),
        true,
        json!({"name":"My plugin"}),
    )
    .await;
    assert_eq!(status, 201);
    assert!(plugin.get("owner_id").is_none());
    let parent = plugin["id"].as_str().unwrap();
    let (_, bobs) = request(
        &router,
        "POST",
        "/plugins",
        Some(&bob),
        true,
        json!({"name":"Bob plugin"}),
    )
    .await;
    let bobs_parent = bobs["id"].as_str().unwrap();
    // Each resource: anonymous, other owner, ownership reassignment, stale version,
    // CSRF-less mutation, list isolation, explicit views, successful mutation and delete.
    for resource in ["plugins", "versions"] {
        let input = if resource == "plugins" {
            json!({"name":"Owned"})
        } else {
            json!({"name":"1.0.0","plugin_id":parent})
        };
        let base = format!("/{resource}");
        assert_eq!(
            request(&router, "POST", &base, None, false, input.clone())
                .await
                .0,
            401
        );
        assert_eq!(
            request(&router, "POST", &base, Some(&alice), false, input.clone())
                .await
                .0,
            403
        );
        let mut reassignment = input.clone();
        reassignment["owner_id"] = json!(Uuid::new_v4());
        assert_eq!(
            request(&router, "POST", &base, Some(&alice), true, reassignment)
                .await
                .0,
            400
        );
        let (status, created) =
            request(&router, "POST", &base, Some(&alice), true, input.clone()).await;
        assert_eq!(status, 201);
        let path = format!("{base}/{}", created["id"].as_str().unwrap());
        assert!(created.get("owner_id").is_none());
        assert!(created.get("password_hash").is_none());
        assert_eq!(
            request(&router, "GET", &path, None, false, Value::Null)
                .await
                .0,
            401
        );
        for (method, body) in [
            ("GET", Value::Null),
            ("PATCH", json!({"name":"stolen","version":1})),
            ("DELETE", json!({"version":1})),
        ] {
            assert_eq!(
                request(&router, method, &path, Some(&bob), true, body)
                    .await
                    .0,
                404
            );
        }
        let (_, listed) = request(&router, "GET", &base, Some(&bob), false, Value::Null).await;
        assert!(
            listed
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["id"] != created["id"])
        );
        assert_eq!(
            request(
                &router,
                "PATCH",
                &path,
                Some(&alice),
                false,
                json!({"name":"bad","version":1})
            )
            .await
            .0,
            403
        );
        assert_eq!(
            request(
                &router,
                "PATCH",
                &path,
                Some(&alice),
                true,
                json!({"name":"bad","version":1,"owner_id":Uuid::new_v4()})
            )
            .await
            .0,
            400
        );
        let (status, updated) = request(
            &router,
            "PATCH",
            &path,
            Some(&alice),
            true,
            json!({"name":"updated","version":1}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(updated["version"], 2);
        assert_eq!(
            request(
                &router,
                "PATCH",
                &path,
                Some(&alice),
                true,
                json!({"name":"stale","version":1})
            )
            .await
            .0,
            409
        );
        assert_eq!(
            request(
                &router,
                "DELETE",
                &path,
                Some(&alice),
                true,
                json!({"version":1})
            )
            .await
            .0,
            409
        );
        assert_eq!(
            request(&router, "GET", &path, Some(&alice), false, Value::Null)
                .await
                .1["name"],
            "updated"
        );
        if resource == "versions" {
            assert_eq!(
                request(
                    &router,
                    "PATCH",
                    &path,
                    Some(&alice),
                    true,
                    json!({"name":"reparent","version":2,"plugin_id":bobs_parent})
                )
                .await
                .0,
                400
            );
        }
        assert_eq!(
            request(
                &router,
                "DELETE",
                &path,
                Some(&alice),
                true,
                json!({"version":2})
            )
            .await
            .0,
            200
        );
        assert_eq!(
            request(&router, "GET", &path, Some(&alice), false, Value::Null)
                .await
                .0,
            404
        );
    }
    // Child authority comes from the stored parent owner, never the supplied parent ID.
    assert_eq!(
        request(
            &router,
            "POST",
            "/versions",
            Some(&alice),
            true,
            json!({"name":"attack","plugin_id":bobs_parent})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/versions",
            Some(&alice),
            true,
            json!({"name":"missing","plugin_id":Uuid::new_v4()})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(
            &router,
            "GET",
            "/versions",
            Some(&alice),
            false,
            Value::Null
        )
        .await
        .1,
        json!([])
    );
    // A scoped bearer can read but cannot borrow the session's write authority.
    let identity = auth
        .authenticate(
            &[(
                "Cookie".into(),
                format!("aor_dev_session={}", alice.session.expose()).into_bytes(),
            )],
            "GET",
        )
        .await
        .unwrap()
        .unwrap();
    let token = auth
        .create_api_token(identity.principal(), vec!["plugins:read".into()], 60)
        .await
        .unwrap();
    for (method, status) in [("GET", 200), ("POST", 404)] {
        let response = router
            .handle(Request {
                method: method.into(),
                target: "/plugins".into(),
                headers: vec![
                    (
                        "Authorization".into(),
                        format!("Bearer {}", token.expose()).into_bytes(),
                    ),
                    ("Content-Type".into(), b"application/json".to_vec()),
                ],
                body: Body::from_bytes(b"{\"name\":\"not allowed\"}".to_vec(), 4096).unwrap(),
            })
            .await;
        assert_eq!(response.status, status);
    }
    // A previously issued capability is invalid after account-wide revocation.
    let scope = aor_policy::owner::<aor_registry::plugins::Plugin, aor_policy::Create>(Some(
        identity.principal(),
    ))
    .unwrap();
    auth.revoke_all(identity.principal()).await.unwrap();
    assert_eq!(
        aor_registry::plugins::create(
            &aor_registry::State {
                pool,
                auth,
                mail_spool: None
            },
            scope,
            aor_registry::plugins::Input {
                name: "revoked".into()
            }
        )
        .await
        .err(),
        Some(aor_router::AppError::Unauthenticated)
    );
}
#[tokio::test]
async fn sqlite_resource_matrix() {
    let file = std::env::temp_dir().join(format!("aor-registry-{}.sqlite", Uuid::new_v4()));
    matrix(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_resource_matrix() {
    matrix(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
    )
    .await;
}
