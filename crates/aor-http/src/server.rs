use crate::{Chunk, ChunkDecoder, Framing, Limits, ParseError, parse_head};
use std::{future::Future, io, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, UnixListener},
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
    time::timeout,
};

/// Concrete, bounded stream. Dropping it causes the transport to drain and validate the body.
pub struct Body {
    rx: mpsc::Receiver<Result<Vec<u8>, ParseError>>,
    completion: Option<oneshot::Receiver<Result<(), ParseError>>>,
    limit: usize,
}
impl Body {
    pub async fn next(&mut self) -> Option<Result<Vec<u8>, ParseError>> {
        if let Some(part) = self.rx.recv().await {
            return Some(part);
        }
        if let Some(done) = self.completion.take() {
            if let Err(e) = done.await.unwrap_or(Err(ParseError::IncompleteBody)) {
                return Some(Err(e));
            }
        }
        None
    }
    pub async fn collect(mut self) -> Result<Vec<u8>, ParseError> {
        let mut out = Vec::new();
        while let Some(part) = self.next().await {
            let part = part?;
            if part.len() > self.limit.saturating_sub(out.len()) {
                return Err(ParseError::BodyLimit);
            }
            out.extend_from_slice(&part);
        }
        Ok(out)
    }
}
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Body,
}
impl Request {
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
    }
}
pub enum ResponseBody {
    Bytes(Vec<u8>),
    Stream(mpsc::Receiver<io::Result<Vec<u8>>>),
}
pub struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: ResponseBody,
}
impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: ResponseBody::Bytes(body.into()),
        }
    }
    pub fn stream(status: u16, body: mpsc::Receiver<io::Result<Vec<u8>>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: ResponseBody::Stream(body),
        }
    }
    /// Framing belongs to the transport; callers cannot inject it or split a response.
    pub fn header(mut self, name: &str, value: &str) -> Result<Self, io::Error> {
        if name.is_empty()
            || !name.bytes().all(crate::is_token)
            || value.bytes().any(|b| b < 32 || b == 127)
            || [
                "content-length",
                "transfer-encoding",
                "connection",
                "trailer",
                "upgrade",
            ]
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
            || self.headers.len() >= 64
            || name.len() + value.len() > 8192
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid response header",
            ));
        }
        self.headers.push((name.to_owned(), value.to_owned()));
        Ok(self)
    }
}
fn io_error(e: ParseError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        417 => "Expectation Failed",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Response",
    }
}
async fn write_response<S: AsyncWrite + Unpin>(
    s: &mut S,
    mut r: Response,
    head: bool,
    close: bool,
    limits: &Limits,
) -> io::Result<()> {
    // Informational responses and switching protocols are transport-only.
    if !(200..=599).contains(&r.status) {
        r = Response::new(500, b"HTTP_RESPONSE_STATUS".to_vec());
    }
    let no_body = r.status == 204 || r.status == 304;
    let mut bytes = format!("HTTP/1.1 {} {}\r\n", r.status, reason(r.status));
    for (k, v) in &r.headers {
        bytes.push_str(k);
        bytes.push_str(": ");
        bytes.push_str(v);
        bytes.push_str("\r\n");
    }
    if !no_body {
        match &r.body {
            ResponseBody::Bytes(b) => bytes.push_str(&format!("Content-Length: {}\r\n", b.len())),
            ResponseBody::Stream(_) => bytes.push_str("Transfer-Encoding: chunked\r\n"),
        }
    }
    if close {
        bytes.push_str("Connection: close\r\n");
    }
    bytes.push_str("\r\n");
    timeout(limits.write_timeout, s.write_all(bytes.as_bytes()))
        .await
        .map_err(|_| io_error(ParseError::Timeout))??;
    if head || no_body {
        return Ok(());
    }
    match r.body {
        ResponseBody::Bytes(bytes) => timeout(limits.write_timeout, s.write_all(&bytes))
            .await
            .map_err(|_| io_error(ParseError::Timeout))??,
        ResponseBody::Stream(mut rx) => {
            while let Some(chunk) = timeout(limits.write_timeout, rx.recv())
                .await
                .map_err(|_| io_error(ParseError::Timeout))?
            {
                let chunk = chunk?;
                if chunk.is_empty() {
                    continue;
                }
                // A response producer is trusted application code, but still cannot hold the socket forever.
                timeout(limits.write_timeout, async {
                    s.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                        .await?;
                    s.write_all(&chunk).await?;
                    s.write_all(b"\r\n").await
                })
                .await
                .map_err(|_| io_error(ParseError::Timeout))??;
            }
            timeout(limits.write_timeout, s.write_all(b"0\r\n\r\n"))
                .await
                .map_err(|_| io_error(ParseError::Timeout))??;
        }
    }
    Ok(())
}
async fn error_response<S: AsyncWrite + Unpin>(
    s: &mut S,
    e: ParseError,
    limits: &Limits,
) -> io::Result<()> {
    write_response(
        s,
        Response::new(e.status(), e.code().as_bytes().to_vec()),
        false,
        true,
        limits,
    )
    .await
}
async fn read_more<S: AsyncRead + Unpin>(
    s: &mut S,
    buffer: &mut Vec<u8>,
    max: usize,
) -> io::Result<usize> {
    let mut scratch = [0u8; 8192];
    let cap = scratch.len().min(max);
    let n = s.read(&mut scratch[..cap]).await?;
    buffer.extend_from_slice(&scratch[..n]);
    Ok(n)
}
async fn pump<S: AsyncRead + Unpin>(
    s: &mut S,
    buf: &mut Vec<u8>,
    framing: Framing,
    tx: &mpsc::Sender<Result<Vec<u8>, ParseError>>,
    limits: &Limits,
) -> Result<(), ParseError> {
    match framing {
        Framing::Empty => Ok(()),
        Framing::Length(mut left) => {
            while left > 0 {
                if buf.is_empty()
                    && read_more(s, buf, 8192)
                        .await
                        .map_err(|_| ParseError::IncompleteBody)?
                        == 0
                {
                    return Err(ParseError::IncompleteBody);
                }
                let n = left.min(buf.len());
                if !tx.is_closed() {
                    let _ = tx.send(Ok(buf[..n].to_vec())).await;
                }
                buf.drain(..n);
                left -= n;
            }
            Ok(())
        }
        Framing::Chunked => {
            let mut decoder = ChunkDecoder::default();
            loop {
                let consumed = match decoder.step(buf, limits)? {
                    Chunk::NeedMore => {
                        if buf.len() > limits.chunk_line.max(8192) {
                            return Err(ParseError::ChunkLimit);
                        }
                        if read_more(s, buf, 8192)
                            .await
                            .map_err(|_| ParseError::IncompleteBody)?
                            == 0
                        {
                            return Err(ParseError::IncompleteBody);
                        }
                        continue;
                    }
                    Chunk::Data { bytes, consumed } => {
                        if !tx.is_closed() {
                            let _ = tx.send(Ok(bytes.to_vec())).await;
                        }
                        consumed
                    }
                    Chunk::Progress(n) => n,
                    Chunk::End(n) => {
                        buf.drain(..n);
                        return Ok(());
                    }
                };
                buf.drain(..consumed);
            }
        }
    }
}

/// Generic transport entry point, also used by byte-exact socket tests.
pub async fn connection<S, F, Fut>(
    mut stream: S,
    handler: Arc<F>,
    limits: Limits,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: Fn(Request) -> Fut + Send + Sync,
    Fut: Future<Output = Response> + Send,
{
    limits.validate().map_err(io_error)?;
    let mut buffer = Vec::with_capacity(8192);
    for count in 0..limits.requests_per_connection {
        if *shutdown.borrow() {
            break;
        }
        if buffer.is_empty() {
            tokio::select! {
                _ = shutdown.changed() => break,
                read = timeout(limits.idle_timeout, read_more(&mut stream, &mut buffer, 8192)) => {
                    match read { Ok(Ok(0)) => break, Ok(Ok(_)) => {}, Ok(Err(e)) => return Err(e), Err(_) => break }
                }
            }
        }
        let parsed = timeout(limits.header_timeout, async {
            loop {
                match parse_head(&buffer, &limits)? {
                    Some(h) => {
                        return Ok((
                            h.method.to_owned(),
                            h.target.to_owned(),
                            h.headers()
                                .iter()
                                .map(|h| {
                                    (
                                        String::from_utf8_lossy(h.name).into_owned(),
                                        h.value.to_vec(),
                                    )
                                })
                                .collect(),
                            h.framing,
                            h.close,
                            h.expect_continue,
                            h.consumed,
                        ));
                    }
                    None => {
                        let room = limits.header_bytes.saturating_sub(buffer.len());
                        if room == 0 {
                            return Err(ParseError::HeaderLimit);
                        }
                        if read_more(&mut stream, &mut buffer, room)
                            .await
                            .map_err(|_| ParseError::IncompleteBody)?
                            == 0
                        {
                            return Err(ParseError::IncompleteBody);
                        }
                    }
                }
            }
        })
        .await
        .unwrap_or(Err(ParseError::Timeout));
        let (method, target, headers, framing, close, expect, consumed) = match parsed {
            Ok(parts) => parts,
            Err(e) => {
                error_response(&mut stream, e, &limits).await?;
                break;
            }
        };
        buffer.drain(..consumed);
        if expect {
            timeout(
                limits.write_timeout,
                stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n"),
            )
            .await
            .map_err(|_| io_error(ParseError::Timeout))??;
        }
        let is_head = method == "HEAD";
        let (tx, rx) = mpsc::channel(2);
        let (completed, completion) = oneshot::channel();
        let request = Request {
            method,
            target,
            headers,
            body: Body {
                rx,
                completion: Some(completion),
                limit: limits.body_bytes,
            },
        };
        let process = async {
            let read = async {
                let result = timeout(
                    limits.body_timeout,
                    pump(&mut stream, &mut buffer, framing, &tx, &limits),
                )
                .await
                .unwrap_or(Err(ParseError::Timeout));
                let _ = completed.send(result);
                drop(tx);
                result
            };
            tokio::join!(read, handler(request))
        };
        let (body, response) = match timeout(limits.handler_timeout, process).await {
            Ok(pair) => pair,
            Err(_) => {
                write_response(
                    &mut stream,
                    Response::new(503, b"HTTP_HANDLER_TIMEOUT".to_vec()),
                    is_head,
                    true,
                    &limits,
                )
                .await?;
                break;
            }
        };
        if let Err(e) = body {
            error_response(&mut stream, e, &limits).await?;
            break;
        }
        let close = close || count + 1 == limits.requests_per_connection || *shutdown.borrow();
        write_response(&mut stream, response, is_head, close, &limits).await?;
        if close {
            break;
        }
    }
    timeout(limits.write_timeout, stream.shutdown())
        .await
        .map_err(|_| io_error(ParseError::Timeout))?
}

pub enum Listener {
    Tcp(TcpListener),
    Unix(UnixListener),
}
impl Listener {
    /// Remote addresses are rejected. Put the reverse proxy on the same host.
    pub async fn tcp(address: std::net::SocketAddr) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "AoR binds loopback only",
            ));
        }
        Ok(Self::Tcp(TcpListener::bind(address).await?))
    }
    /// Never unlinks an existing path. The owner must resolve stale sockets explicitly.
    pub fn unix(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        Ok(Self::Unix(UnixListener::bind(path)?))
    }
}
/// Stops accepts immediately on shutdown, then drains or aborts the remaining tasks.
pub async fn serve<F, Fut, Stop>(
    listener: Listener,
    limits: Limits,
    handler: F,
    stop: Stop,
) -> io::Result<()>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
    Stop: Future<Output = ()>,
{
    limits.validate().map_err(io_error)?;
    if let Listener::Tcp(tcp) = &listener {
        if !tcp.local_addr()?.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "AoR binds loopback only",
            ));
        }
    }
    let handler = Arc::new(handler);
    let sem = Arc::new(Semaphore::new(limits.connections));
    let (shutdown, _) = watch::channel(false);
    let mut tasks = JoinSet::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            permit = sem.clone().acquire_owned() => {
                let permit = permit.map_err(|_| io::Error::other("connection semaphore closed"))?;
                let handler = handler.clone(); let limits = limits.clone(); let receiver = shutdown.subscribe();
                match &listener {
                    Listener::Tcp(listener) => {
                        let (s,_) = tokio::select! { biased; _ = &mut stop => break, a = listener.accept() => a? };
                        tasks.spawn(async move { let _permit = permit; connection(s, handler, limits, receiver).await });
                    }
                    Listener::Unix(listener) => {
                        let (s,_) = tokio::select! { biased; _ = &mut stop => break, a = listener.accept() => a? };
                        tasks.spawn(async move { let _permit = permit; connection(s, handler, limits, receiver).await });
                    }
                }
            }
        }
    }
    shutdown.send_replace(true);
    if timeout(limits.drain_timeout, async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    Ok(())
}
