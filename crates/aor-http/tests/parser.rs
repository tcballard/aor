use aor_http::*;

#[test]
fn canonical_roundtrip_and_pipeline_boundary() {
    let raw = b"POST /hello?x=1 HTTP/1.1\r\nHost:\tlocalhost \t\r\nContent-Length: 3\r\nConnection: keep-alive, close\r\n\r\nabcGET / HTTP/1.1\r\nHost: x\r\n\r\n";
    let h = parse_head(raw, &Limits::default()).unwrap().unwrap();
    assert_eq!(&raw[h.consumed..h.consumed + 3], b"abc");
    assert_eq!(h.framing, Framing::Length(3));
    assert!(h.close);
    let canonical = h.canonical();
    let h2 = parse_head(&canonical, &Limits::default()).unwrap().unwrap();
    assert_eq!(h2.canonical(), canonical);
    assert_eq!(h2.method, h.method);
    assert_eq!(h2.target, h.target);
    assert_eq!(h2.framing, h.framing);
}
#[test]
fn every_prefix_is_incomplete() {
    let raw = b"POST / HTTP/1.1\r\nHost: [::1]:80\r\nContent-Length: 10\r\n\r\n";
    for n in 0..raw.len() {
        assert!(
            parse_head(&raw[..n], &Limits::default()).unwrap().is_none(),
            "prefix {n}"
        );
    }
    assert!(parse_head(raw, &Limits::default()).unwrap().is_some());
}
#[test]
fn smuggling_corpus() {
    let cases: &[(&[u8], ParseError)] = &[
        (
            b"Content-Length: 1\r\nTransfer-Encoding: chunked",
            ParseError::AmbiguousFraming,
        ),
        (
            b"Transfer-Encoding: chunked\r\nContent-Length: 1",
            ParseError::AmbiguousFraming,
        ),
        (
            b"Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked",
            ParseError::AmbiguousFraming,
        ),
        (
            b"Content-Length: 1\r\nContent-Length: 2",
            ParseError::AmbiguousFraming,
        ),
        (
            b"Content-Length: 1\r\nContent-Length: 1",
            ParseError::AmbiguousFraming,
        ),
        (b"Content-Length: 1, 1", ParseError::ContentLength),
        (b"Content-Length: +1", ParseError::ContentLength),
        (
            b"Content-Length: 184467440737095516160",
            ParseError::ContentLength,
        ),
        (b"Transfer-Encoding : chunked", ParseError::HeaderSyntax),
        (
            b"Transfer-Encoding: chunked, chunked",
            ParseError::TransferEncoding,
        ),
        (
            b"Transfer-Encoding: gzip, chunked",
            ParseError::TransferEncoding,
        ),
        (
            b"Transfer-Encoding:\tchunked\r\n x: y",
            ParseError::HeaderSyntax,
        ),
        (b"X: a\rb", ParseError::LineEnding),
        (b"X: a\nb", ParseError::LineEnding),
        (b"X: a\0b", ParseError::HeaderValue),
        (b"X: a\x7fb", ParseError::HeaderValue),
        (b"Host: evil", ParseError::Host),
        (b"Connection: Content-Length", ParseError::Unsupported),
        (b"Trailer: Content-Length", ParseError::TrailersUnsupported),
    ];
    for (header, error) in cases {
        let mut raw = b"POST / HTTP/1.1\r\nHost: localhost\r\n".to_vec();
        raw.extend_from_slice(header);
        raw.extend_from_slice(b"\r\n\r\n");
        assert_eq!(
            parse_head(&raw, &Limits::default()).unwrap_err(),
            *error,
            "{header:?}"
        );
    }
}
#[test]
fn targets_hosts_expectations_and_limits() {
    for request in [
        "G ET / HTTP/1.1\r\nHost: x\r\n\r\n",
        "GET http://evil/ HTTP/1.1\r\nHost: x\r\n\r\n",
        "GET /%xy HTTP/1.1\r\nHost: x\r\n\r\n",
        "GET / HTTP/1.1\r\n\r\n",
        "GET / HTTP/1.1\r\nHost: x:99999\r\n\r\n",
        "GET / HTTP/1.1\r\nHost: x@y\r\n\r\n",
    ] {
        assert!(
            parse_head(request.as_bytes(), &Limits::default()).is_err(),
            "{request:?}"
        );
    }
    let limits = Limits {
        body_bytes: 3,
        ..Limits::default()
    };
    assert_eq!(
        parse_head(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nExpect: 100-continue\r\n\r\n",
            &limits
        )
        .unwrap_err(),
        ParseError::BodyLimit
    );
    let limits = Limits {
        header_count: 1,
        ..Limits::default()
    };
    assert_eq!(
        parse_head(b"GET / HTTP/1.1\r\nHost: x\r\nX: y\r\n\r\n", &limits).unwrap_err(),
        ParseError::HeaderLimit
    );
}
fn decode(input: &[u8], fragment: usize, limits: &Limits) -> Result<(Vec<u8>, usize), ParseError> {
    let mut d = ChunkDecoder::default();
    let mut out = Vec::new();
    let mut offset = 0;
    let mut end = 0;
    loop {
        match d.step(&input[offset..end], limits)? {
            Chunk::NeedMore => {
                if end == input.len() {
                    return Err(ParseError::IncompleteBody);
                }
                end = (end + fragment).min(input.len());
            }
            Chunk::Data { bytes, consumed } => {
                out.extend_from_slice(bytes);
                offset += consumed;
            }
            Chunk::Progress(n) => offset += n,
            Chunk::End(n) => return Ok((out, offset + n)),
        }
    }
}
#[test]
fn chunked_fragmentation_and_pipeline() {
    let bytes = b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\nGET / HTTP/1.1\r\n";
    for n in 1..bytes.len() {
        let (out, end) = decode(bytes, n, &Limits::default()).unwrap();
        assert_eq!(out, b"abcde");
        assert_eq!(&bytes[end..], b"GET / HTTP/1.1\r\n");
    }
}
#[test]
fn chunked_rejects_extensions_trailers_overflow_and_limits() {
    for bytes in [
        b"+1\r\na\r\n0\r\n\r\n".as_slice(),
        b"1;x=y\r\na\r\n0\r\n\r\n",
        b"0\r\nContent-Length: 2\r\n\r\n",
        b"ffffffffffffffffffffffff\r\n",
        b"1\r\naX\n",
        b"1\r\na\r\n",
    ] {
        assert!(decode(bytes, 1, &Limits::default()).is_err());
    }
    assert_eq!(
        decode(
            b"4\r\nabcd\r\n0\r\n\r\n",
            1,
            &Limits {
                body_bytes: 3,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        ParseError::BodyLimit
    );
    assert_eq!(
        decode(
            b"1\r\na\r\n1\r\nb\r\n0\r\n\r\n",
            1,
            &Limits {
                max_chunks: 2,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        ParseError::ChunkLimit
    );
}
#[test]
fn deterministic_arbitrary_bytes_never_panic_and_canonicalise() {
    let mut seed = 123456789u64;
    for len in 0..2048 {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        if let Ok(Some(head)) = parse_head(&bytes, &Limits::default()) {
            let canonical = head.canonical();
            let parsed = parse_head(&canonical, &Limits::default()).unwrap().unwrap();
            assert_eq!(parsed.canonical(), canonical);
            assert_eq!(parsed.framing, head.framing);
        }
        let _ = decode(&bytes, 7, &Limits::default());
        let _ = tokens(&bytes);
    }
}
