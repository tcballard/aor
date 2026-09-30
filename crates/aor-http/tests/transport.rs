use aor_http::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UnixStream},
    sync::{mpsc, oneshot},
};
async fn start(
    limits: Limits,
) -> (
    std::net::SocketAddr,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, rx) = oneshot::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let task = tokio::spawn(async move {
        serve(
            Listener::Tcp(listener),
            limits,
            move |r| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    if r.target == "/stream" {
                        let (tx, rx) = mpsc::channel(2);
                        tokio::spawn(async move {
                            tx.send(Ok(b"abc".to_vec())).await.unwrap();
                            tx.send(Ok(b"de".to_vec())).await.unwrap();
                        });
                        Response::stream(200, rx)
                    } else {
                        match r.body.collect().await {
                            Ok(bytes) => Response::new(200, bytes),
                            Err(e) => Response::new(e.status(), e.code().as_bytes().to_vec()),
                        }
                    }
                }
            },
            async {
                let _ = rx.await;
            },
        )
        .await
        .unwrap();
    });
    (addr, stop, task, calls)
}
async fn exchange(addr: std::net::SocketAddr, bytes: &[u8]) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(bytes).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut out))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(out).unwrap()
}
#[tokio::test]
async fn pipelined_chunked_then_content_length() {
    let (a, stop, t, c) = start(Limits::default()).await;
    let response=exchange(a,b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\nPOST / HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\nde").await;
    assert_eq!(response.matches("HTTP/1.1 200").count(), 2);
    assert!(response.contains("\r\n\r\nabcHTTP/1.1"));
    assert!(response.ends_with("\r\n\r\nde"));
    assert_eq!(c.load(Ordering::SeqCst), 2);
    stop.send(()).unwrap();
    t.await.unwrap();
}
#[tokio::test]
async fn smuggling_is_closed_before_handler() {
    let (a, stop, t, c) = start(Limits::default()).await;
    let response=exchange(a,b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(response.starts_with("HTTP/1.1 400"));
    assert_eq!(response.matches("HTTP/1.1").count(), 1);
    assert_eq!(c.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    t.await.unwrap();
}
#[tokio::test]
async fn expect_continue_and_preflight_rejection() {
    let (a, stop, t, _) = start(Limits {
        body_bytes: 4,
        ..Limits::default()
    })
    .await;
    let mut s = TcpStream::connect(a).await.unwrap();
    s.write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut interim = [0u8; 25];
    s.read_exact(&mut interim).await.unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    s.write_all(b"abc").await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.ends_with("abc"));
    let out = exchange(
        a,
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\n",
    )
    .await;
    assert!(out.starts_with("HTTP/1.1 413"));
    assert!(!out.contains("100 Continue"));
    stop.send(()).unwrap();
    t.await.unwrap();
}
#[tokio::test]
async fn slow_headers_body_and_keepalive_are_bounded() {
    let (a, stop, t, _) = start(Limits {
        header_timeout: Duration::from_millis(40),
        body_timeout: Duration::from_millis(40),
        idle_timeout: Duration::from_millis(40),
        ..Limits::default()
    })
    .await;
    assert!(
        exchange(a, b"GET / HTTP/1.1\r\nHost:")
            .await
            .starts_with("HTTP/1.1 408")
    );
    assert!(
        exchange(
            a,
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\na"
        )
        .await
        .starts_with("HTTP/1.1 408")
    );
    assert_eq!(exchange(a, b"").await, "");
    stop.send(()).unwrap();
    t.await.unwrap();
}
#[tokio::test]
async fn streaming_response_head_and_connection_cap() {
    let (a, stop, t, _) = start(Limits {
        requests_per_connection: 1,
        ..Limits::default()
    })
    .await;
    let out = exchange(a, b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(out.contains("Transfer-Encoding: chunked\r\n"));
    assert!(out.ends_with("3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"));
    assert!(out.contains("Connection: close"));
    let out = exchange(
        a,
        b"HEAD / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nabc",
    )
    .await;
    assert!(out.contains("Content-Length: 3\r\n"));
    assert!(out.ends_with("\r\n\r\n"));
    stop.send(()).unwrap();
    t.await.unwrap();
}
#[tokio::test]
async fn graceful_shutdown_drains_handler_and_marks_close() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    let (stop, rx) = oneshot::channel();
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = entered.clone();
    let t = tokio::spawn(serve(
        Listener::Tcp(l),
        Limits::default(),
        move |_r| {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                tokio::time::sleep(Duration::from_millis(40)).await;
                Response::new(200, b"done".to_vec())
            }
        },
        async {
            let _ = rx.await;
        },
    ));
    let mut s = TcpStream::connect(a).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    entered.notified().await;
    stop.send(()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.contains("Connection: close"));
    assert!(out.ends_with("done"));
    t.await.unwrap().unwrap();
}
#[tokio::test]
async fn unix_socket_roundtrip() {
    let path = std::env::temp_dir().join(format!("aor-test-{}.sock", std::process::id()));
    let listener = Listener::unix(&path).unwrap();
    let (stop, rx) = oneshot::channel();
    let t = tokio::spawn(serve(
        listener,
        Limits::default(),
        |_| async { Response::new(200, b"unix".to_vec()) },
        async {
            let _ = rx.await;
        },
    ));
    let mut s = UnixStream::connect(&path).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.ends_with("unix"));
    stop.send(()).unwrap();
    t.await.unwrap().unwrap();
    std::fs::remove_file(path).unwrap();
}
#[test]
fn response_injection_is_rejected() {
    assert!(
        Response::new(200, vec![])
            .header("X", "ok\r\nInjected: yes")
            .is_err()
    );
    assert!(
        Response::new(200, vec![])
            .header("Content-Length", "9")
            .is_err()
    );
}
