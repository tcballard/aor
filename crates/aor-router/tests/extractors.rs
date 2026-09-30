use aor_http::{Limits, Response};
use aor_router::*;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
route_path!(ShowVersionPath {
    slug: Slug,
    version: SemVer
});
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    limit: u32,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    name: String,
}
#[handler]
async fn show(
    Query(q): Query<Search>,
    id: RequestId,
    Path(path): Path<ShowVersionPath>,
) -> Result<Response, AppError> {
    Ok(Response::new(
        200,
        format!(
            "{} {} {} {} {}",
            path.slug, path.version, q.limit, q.enabled, id.0
        ),
    ))
}
#[handler]
async fn json_input(
    Json(input): Json<Input>,
    Query(q): Query<Search>,
    _: RequestId,
) -> Result<Response, AppError> {
    Ok(Response::new(200, format!("{} {}", input.name, q.limit)))
}
#[handler]
async fn form_input(
    _: RequestId,
    Form(input): Form<Input>,
    Query(q): Query<Search>,
) -> Result<Response, AppError> {
    Ok(Response::new(200, format!("{} {}", input.name, q.limit)))
}
#[handler]
async fn unavailable(_: Principal) -> Result<Response, AppError> {
    panic!("principal must not reach handler before Level 3")
}
async fn exchange(router: Arc<Router>, request: &str) -> String {
    let (mut client, server) = tokio::io::duplex(16384);
    let (_shutdown, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        aor_http::connection(
            server,
            Arc::new(move |r| {
                let router = router.clone();
                async move { router.handle(r).await }
            }),
            Limits::default(),
            rx,
        )
        .await
        .unwrap();
    });
    client.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    task.await.unwrap();
    String::from_utf8(out).unwrap()
}
#[tokio::test]
async fn extracts_in_arbitrary_order_and_denies_bad_inputs() {
    let router = Arc::new(
        Router::new(vec![
            route!(GET "/plugins/{slug}/{version}"=>show),
            route!(POST "/json"=>json_input),
            route!(POST "/form"=>form_input),
            route!(GET "/private"=>unavailable),
        ])
        .unwrap(),
    );
    let get =
        |target: &str| format!("GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let out = exchange(
        router.clone(),
        &get("/plugins/a-plugin/1.2.3-beta.1+build?limit=12&enabled=true"),
    )
    .await;
    assert!(out.starts_with("HTTP/1.1 200"));
    assert!(out.contains("a-plugin 1.2.3-beta.1+build 12 true aor-"));
    for target in [
        "/plugins/a-plugin/01.2.3?limit=1&enabled=true",
        "/plugins/UPPER/1.2.3?limit=1&enabled=true",
        "/plugins/good/1.2.3?limit=bad&enabled=true",
        "/plugins/good/1.2.3?limit=1&limit=2&enabled=true",
    ] {
        assert!(
            exchange(router.clone(), &get(target))
                .await
                .starts_with("HTTP/1.1 400"),
            "{target}"
        )
    }
    for (path, media, body, expected) in [
        ("json", "application/json", r#"{"name":"Ada"}"#, "Ada 2"),
        (
            "form",
            "application/x-www-form-urlencoded",
            "name=Ada+Lovelace",
            "Ada Lovelace 2",
        ),
    ] {
        let request = format!(
            "POST /{path}?limit=2&enabled=false HTTP/1.1\r\nHost: x\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let out = exchange(router.clone(), &request).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.ends_with(expected));
    }
    assert!(
        exchange(router, &get("/private"))
            .await
            .contains("AUTH_NOT_IMPLEMENTED")
    );
}
#[tokio::test]
async fn hooks_cannot_convert_denials_or_override_security_headers() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let before = events.clone();
    let after = events.clone();
    let router = Arc::new(
        Router::new(vec![
            route!(GET "/private"=>unavailable),
            route!(GET "/plugins/{slug}/{version}"=>show),
        ])
        .unwrap()
        .before_route(move |_, _| {
            before.lock().unwrap().push("before");
            Ok(())
        })
        .after_handler(move |trace, response| {
            assert!(trace.route.is_some());
            after.lock().unwrap().push("after");
            response
                .header("X-Request-Id", "spoofed")
                .unwrap()
                .header("Content-Security-Policy", "default-src *")
                .unwrap()
        }),
    );
    let out=exchange(router.clone(),"GET /plugins/demo/1.0.0?limit=1&enabled=true HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert_eq!(*events.lock().unwrap(), vec!["before", "after"]);
    assert!(!out.contains("spoofed"));
    assert!(!out.contains("default-src *"));
    events.lock().unwrap().clear();
    let out = exchange(
        router.clone(),
        "GET /private HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(out.contains("AUTH_NOT_IMPLEMENTED"));
    assert_eq!(*events.lock().unwrap(), vec!["before"]);
    events.lock().unwrap().clear();
    let out=exchange(router,"GET /private HTTP/1.1\r\nHost: x\r\nCookie: session=untrusted\r\nConnection: close\r\n\r\n").await;
    assert!(out.contains("AUTH_NOT_IMPLEMENTED"));
    assert!(events.lock().unwrap().is_empty());
}
#[tokio::test]
async fn excessive_handler_headers_fail_without_panicking() {
    let handler = |_: Context| async {
        let mut response = Response::new(200, "x");
        for _ in 0..64 {
            response = response.header("X-Test", "x").unwrap();
        }
        Ok(response)
    };
    let router = Arc::new(Router::new(vec![route!(GET "/"=>handler)]).unwrap());
    let out = exchange(
        router,
        "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(out.starts_with("HTTP/1.1 500"));
    assert!(out.contains("X-Content-Type-Options: nosniff"));
}
