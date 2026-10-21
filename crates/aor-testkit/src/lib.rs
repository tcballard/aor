//! Exercise the owned HTTP transport and router without a listening socket.
use std::{io, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub async fn exchange(
    router: Arc<aor_router::Router>,
    request: Vec<u8>,
) -> io::Result<(u16, Vec<u8>)> {
    tokio::time::timeout(std::time::Duration::from_secs(15), async move {
        let (mut client, server) = tokio::io::duplex(8192);
        let (_keep, stop) = tokio::sync::watch::channel(false);
        let handler = Arc::new(move |request| {
            let router = router.clone();
            async move { router.handle(request).await }
        });
        let task = tokio::spawn(aor_http::connection(
            server,
            handler,
            aor_http::Limits::default(),
            stop,
        ));
        client.write_all(&request).await?;
        client.shutdown().await?;
        let mut response = Vec::new();
        client
            .take(8 * 1024 * 1024)
            .read_to_end(&mut response)
            .await?;
        task.await.map_err(io::Error::other)??;
        let boundary = response
            .windows(4)
            .position(|b| b == b"\r\n\r\n")
            .ok_or_else(|| io::Error::other("response has no header terminator"))?;
        let head = std::str::from_utf8(&response[..boundary]).map_err(io::Error::other)?;
        let status = head
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| io::Error::other("missing status"))?
            .parse::<u16>()
            .map_err(io::Error::other)?;
        let size = head
            .lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                    .map(|(_, n)| n.trim().parse::<usize>())
            })
            .ok_or_else(|| io::Error::other("matrix expects bounded Content-Length response"))?
            .map_err(io::Error::other)?;
        let body = response[boundary + 4..].to_vec();
        if body.len() != size {
            return Err(io::Error::other("response length mismatch"));
        }
        Ok((status, body))
    })
    .await
    .map_err(io::Error::other)?
}
