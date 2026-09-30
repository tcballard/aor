use aor::{
    http::{Limits, Listener, Response},
    router::{AppError, Context, Route, Router},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
#[tokio::test]
async fn credentials_cannot_reach_a_public_handler_and_errors_are_redacted() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let handler = move |_: Context| {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(Response::new(200, b"ok".to_vec()))
        }
    };
    let fail = |_: Context| async { Err(AppError::Internal) };
    let router = Arc::new(
        Router::new(vec![
            Route::public("POST", "/api/v1/probe", "probe", handler),
            Route::public("GET", "/failure", "fail", fail),
        ])
        .unwrap(),
    );
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = l.local_addr().unwrap();
    let (stop, rx) = oneshot::channel();
    let task = tokio::spawn(aor::http::serve(
        Listener::Tcp(l),
        Limits::default(),
        move |r| {
            let router = router.clone();
            async move { router.handle(r).await }
        },
        async {
            let _ = rx.await;
        },
    ));
    for auth in [
        "Cookie: session=some-token\r\n",
        "Authorization: Bearer some-token\r\n",
    ] {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("POST /api/v1/probe HTTP/1.1\r\nHost: localhost\r\n{auth}X-Request-Id: attacker-id\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        socket.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 503"));
        assert!(out.contains("AUTH_NOT_IMPLEMENTED"));
        assert!(!out.contains("attacker-id"));
        assert!(out.contains("X-Content-Type-Options: nosniff"));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let mut socket = TcpStream::connect(address).await.unwrap();
    socket
        .write_all(b"GET /failure HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    socket.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 500"));
    assert!(out.contains("INTERNAL_ERROR"));
    assert!(!out.contains("panic"));
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
