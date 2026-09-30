use std::fmt;

pub const MAX_HEADERS: usize = 96;

/// Every instance includes finite bounds; zero and oversized header counts are invalid.
#[derive(Clone, Debug)]
pub struct Limits {
    pub request_line: usize,
    pub header_bytes: usize,
    pub header_count: usize,
    pub body_bytes: usize,
    pub chunk_line: usize,
    pub max_chunks: usize,
    pub requests_per_connection: usize,
    pub connections: usize,
    pub header_timeout: std::time::Duration,
    pub body_timeout: std::time::Duration,
    pub idle_timeout: std::time::Duration,
    pub handler_timeout: std::time::Duration,
    pub write_timeout: std::time::Duration,
    pub drain_timeout: std::time::Duration,
}
impl Default for Limits {
    fn default() -> Self {
        use std::time::Duration as D;
        Self {
            request_line: 8192,
            header_bytes: 32768,
            header_count: MAX_HEADERS,
            body_bytes: 8 * 1024 * 1024,
            chunk_line: 1024,
            max_chunks: 65536,
            requests_per_connection: 100,
            connections: 256,
            header_timeout: D::from_secs(10),
            body_timeout: D::from_secs(30),
            idle_timeout: D::from_secs(15),
            handler_timeout: D::from_secs(60),
            write_timeout: D::from_secs(15),
            drain_timeout: D::from_secs(30),
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), ParseError> {
        if self.request_line == 0
            || self.header_bytes < self.request_line
            || self.header_count == 0
            || self.header_count > MAX_HEADERS
            || self.body_bytes == 0
            || self.chunk_line == 0
            || self.max_chunks == 0
            || self.requests_per_connection == 0
            || self.connections == 0
            || [
                self.header_timeout,
                self.body_timeout,
                self.idle_timeout,
                self.handler_timeout,
                self.write_timeout,
                self.drain_timeout,
            ]
            .iter()
            .any(|t| t.is_zero())
        {
            return Err(ParseError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    InvalidLimits,
    RequestLine,
    RequestTarget,
    Version,
    LineEnding,
    HeaderSyntax,
    HeaderValue,
    Host,
    HeaderLimit,
    LineLimit,
    AmbiguousFraming,
    ContentLength,
    TransferEncoding,
    Expectation,
    Unsupported,
    BodyLimit,
    ChunkSyntax,
    ChunkLimit,
    TrailersUnsupported,
    IncompleteBody,
    Timeout,
}
impl ParseError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidLimits => "HTTP_INVALID_LIMITS",
            Self::RequestLine => "HTTP_REQUEST_LINE",
            Self::RequestTarget => "HTTP_REQUEST_TARGET",
            Self::Version => "HTTP_VERSION",
            Self::LineEnding => "HTTP_LINE_ENDING",
            Self::HeaderSyntax => "HTTP_HEADER_SYNTAX",
            Self::HeaderValue => "HTTP_HEADER_VALUE",
            Self::Host => "HTTP_HOST",
            Self::HeaderLimit => "HTTP_HEADER_LIMIT",
            Self::LineLimit => "HTTP_LINE_LIMIT",
            Self::AmbiguousFraming => "HTTP_AMBIGUOUS_FRAMING",
            Self::ContentLength => "HTTP_CONTENT_LENGTH",
            Self::TransferEncoding => "HTTP_TRANSFER_ENCODING",
            Self::Expectation => "HTTP_EXPECTATION",
            Self::Unsupported => "HTTP_UNSUPPORTED",
            Self::BodyLimit => "HTTP_BODY_LIMIT",
            Self::ChunkSyntax => "HTTP_CHUNK_SYNTAX",
            Self::ChunkLimit => "HTTP_CHUNK_LIMIT",
            Self::TrailersUnsupported => "HTTP_TRAILERS_UNSUPPORTED",
            Self::IncompleteBody => "HTTP_INCOMPLETE_BODY",
            Self::Timeout => "HTTP_TIMEOUT",
        }
    }
    pub fn status(self) -> u16 {
        match self {
            Self::HeaderLimit => 431,
            Self::LineLimit => 414,
            Self::BodyLimit | Self::ChunkLimit => 413,
            Self::Timeout => 408,
            Self::Version => 505,
            Self::Expectation => 417,
            Self::Unsupported | Self::TransferEncoding | Self::TrailersUnsupported => 501,
            Self::InvalidLimits => 500,
            _ => 400,
        }
    }
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for ParseError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    Empty,
    Length(usize),
    Chunked,
}
#[derive(Debug)]
pub struct Head<'a> {
    pub method: &'a str,
    pub target: &'a str,
    headers: [Header<'a>; MAX_HEADERS],
    count: usize,
    pub framing: Framing,
    pub close: bool,
    pub expect_continue: bool,
    pub consumed: usize,
}
impl Head<'_> {
    pub fn headers(&self) -> &[Header<'_>] {
        &self.headers[..self.count]
    }
    /// Preserves framing, method and target; canonicalises OWS, header case and CRLF.
    pub fn canonical(&self) -> Vec<u8> {
        let mut out = format!("{} {} HTTP/1.1\r\n", self.method, self.target).into_bytes();
        for h in self.headers() {
            out.extend(h.name.iter().map(u8::to_ascii_lowercase));
            out.extend_from_slice(b":");
            out.extend_from_slice(h.value);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out
    }
}

pub fn is_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
pub fn tokens(value: &[u8]) -> Result<impl Iterator<Item = &[u8]>, ParseError> {
    for part in value.split(|b| *b == b',') {
        let part = trim(part);
        if part.is_empty() || !part.iter().copied().all(is_token) {
            return Err(ParseError::HeaderValue);
        }
    }
    Ok(value.split(|b| *b == b',').map(trim))
}
fn trim(mut s: &[u8]) -> &[u8] {
    while matches!(s.first(), Some(b' ' | b'\t')) {
        s = &s[1..];
    }
    while matches!(s.last(), Some(b' ' | b'\t')) {
        s = &s[..s.len() - 1];
    }
    s
}
/// A strict CRLF line scanner. A trailing CR is incomplete, never accepted as a line end.
fn line(input: &[u8], max: usize, limit: ParseError) -> Result<Option<(&[u8], usize)>, ParseError> {
    for (i, b) in input.iter().enumerate() {
        if i > max {
            return Err(limit);
        }
        match *b {
            b'\n' => return Err(ParseError::LineEnding),
            b'\r' => {
                if i > max {
                    return Err(limit);
                }
                return match input.get(i + 1) {
                    None => Ok(None),
                    Some(b'\n') => Ok(Some((&input[..i], i + 2))),
                    _ => Err(ParseError::LineEnding),
                };
            }
            _ => {}
        }
    }
    if input.len() > max {
        Err(limit)
    } else {
        Ok(None)
    }
}
fn decimal(s: &[u8]) -> Result<usize, ParseError> {
    if s.is_empty() {
        return Err(ParseError::ContentLength);
    }
    s.iter().try_fold(0usize, |n, b| {
        if !b.is_ascii_digit() {
            return Err(ParseError::ContentLength);
        }
        n.checked_mul(10)
            .and_then(|n| n.checked_add((b - b'0') as usize))
            .ok_or(ParseError::ContentLength)
    })
}
fn valid_host(s: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(s) else {
        return false;
    };
    if let Some(rest) = s.strip_prefix('[') {
        let Some((ip, port)) = rest.split_once(']') else {
            return false;
        };
        return ip.parse::<std::net::Ipv6Addr>().is_ok() && valid_port(port);
    }
    let (host, port) = s
        .split_once(':')
        .map_or((s, ""), |(h, p)| (h, &s[h.len()..h.len() + p.len() + 1]));
    !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && valid_port(port)
}
fn valid_port(s: &str) -> bool {
    s.is_empty()
        || s.strip_prefix(':').is_some_and(|p| {
            !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok()
        })
}

/// No allocations. Headers borrow `input`; limits are enforced before exposing a head.
/// Deliberately rejects all duplicate framing fields, including identical Content-Length.
pub fn parse_head<'a>(input: &'a [u8], limits: &Limits) -> Result<Option<Head<'a>>, ParseError> {
    limits.validate()?;
    let Some((request, mut offset)) = line(input, limits.request_line, ParseError::LineLimit)?
    else {
        return Ok(None);
    };
    let mut fields = request.split(|b| *b == b' ');
    let method = fields.next().ok_or(ParseError::RequestLine)?;
    let target = fields.next().ok_or(ParseError::RequestLine)?;
    let version = fields.next().ok_or(ParseError::RequestLine)?;
    if fields.next().is_some() || method.is_empty() || !method.iter().copied().all(is_token) {
        return Err(ParseError::RequestLine);
    }
    if version != b"HTTP/1.1" {
        return Err(ParseError::Version);
    }
    if method == b"CONNECT" || method == b"TRACE" {
        return Err(ParseError::Unsupported);
    }
    if !(target.starts_with(b"/") || (method == b"OPTIONS" && target == b"*"))
        || target
            .iter()
            .any(|b| !(0x21..=0x7e).contains(b) || *b == b'#' || *b == b'\\')
    {
        return Err(ParseError::RequestTarget);
    }
    // Reject malformed percent escapes here, before router interpretation.
    let mut i = 0;
    while i < target.len() {
        if target[i] == b'%' {
            if i + 2 >= target.len() || !target[i + 1..i + 3].iter().all(u8::is_ascii_hexdigit) {
                return Err(ParseError::RequestTarget);
            }
            i += 2;
        }
        i += 1;
    }
    let mut head = Head {
        method: std::str::from_utf8(method).map_err(|_| ParseError::RequestLine)?,
        target: std::str::from_utf8(target).map_err(|_| ParseError::RequestTarget)?,
        headers: [Header::default(); MAX_HEADERS],
        count: 0,
        framing: Framing::Empty,
        close: false,
        expect_continue: false,
        consumed: 0,
    };
    let (mut host, mut length, mut transfer, mut expect) = (false, None, false, false);
    loop {
        if offset >= limits.header_bytes {
            return Err(ParseError::HeaderLimit);
        }
        let Some((raw, size)) = line(
            &input[offset..],
            limits.header_bytes - offset,
            ParseError::HeaderLimit,
        )?
        else {
            return if input.len() >= limits.header_bytes {
                Err(ParseError::HeaderLimit)
            } else {
                Ok(None)
            };
        };
        offset += size;
        if offset > limits.header_bytes {
            return Err(ParseError::HeaderLimit);
        }
        if raw.is_empty() {
            break;
        }
        if head.count == limits.header_count {
            return Err(ParseError::HeaderLimit);
        }
        let colon = raw
            .iter()
            .position(|b| *b == b':')
            .ok_or(ParseError::HeaderSyntax)?;
        let name = &raw[..colon];
        if name.is_empty() || !name.iter().copied().all(is_token) {
            return Err(ParseError::HeaderSyntax);
        }
        let value = trim(&raw[colon + 1..]);
        if value
            .iter()
            .any(|b| (*b < 0x20 && *b != b'\t') || *b == 0x7f)
        {
            return Err(ParseError::HeaderValue);
        }
        if name.eq_ignore_ascii_case(b"host") {
            if host || !valid_host(value) {
                return Err(ParseError::Host);
            }
            host = true;
        } else if name.eq_ignore_ascii_case(b"content-length") {
            if length.is_some() || transfer {
                return Err(ParseError::AmbiguousFraming);
            }
            length = Some(decimal(value)?);
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            if transfer || length.is_some() {
                return Err(ParseError::AmbiguousFraming);
            }
            if !value.eq_ignore_ascii_case(b"chunked") {
                return Err(ParseError::TransferEncoding);
            }
            transfer = true;
        } else if name.eq_ignore_ascii_case(b"connection") {
            for token in tokens(value)? {
                if token.eq_ignore_ascii_case(b"close") {
                    head.close = true;
                } else if !token.eq_ignore_ascii_case(b"keep-alive") {
                    return Err(ParseError::Unsupported);
                }
            }
        } else if name.eq_ignore_ascii_case(b"expect") {
            if expect || !value.eq_ignore_ascii_case(b"100-continue") {
                return Err(ParseError::Expectation);
            }
            expect = true;
        } else if name.eq_ignore_ascii_case(b"upgrade") {
            return Err(ParseError::Unsupported);
        } else if name.eq_ignore_ascii_case(b"trailer") {
            return Err(ParseError::TrailersUnsupported);
        }
        head.headers[head.count] = Header { name, value };
        head.count += 1;
    }
    if !host {
        return Err(ParseError::Host);
    }
    if length.is_some_and(|n| n > limits.body_bytes) {
        return Err(ParseError::BodyLimit);
    }
    head.framing = if transfer {
        Framing::Chunked
    } else {
        length.map_or(Framing::Empty, Framing::Length)
    };
    head.expect_continue =
        expect && head.framing != Framing::Empty && head.framing != Framing::Length(0);
    head.consumed = offset;
    Ok(Some(head))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkState {
    Size,
    Data(usize),
    DataEnd,
    FinalEnd,
    Done,
}
#[derive(Debug)]
pub struct ChunkDecoder {
    state: ChunkState,
    total: usize,
    chunks: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Chunk<'a> {
    NeedMore,
    Data { bytes: &'a [u8], consumed: usize },
    Progress(usize),
    End(usize),
}
impl Default for ChunkDecoder {
    fn default() -> Self {
        Self {
            state: ChunkState::Size,
            total: 0,
            chunks: 0,
        }
    }
}
impl ChunkDecoder {
    /// Incremental decoding; consumed bytes never include the following pipelined request.
    /// Extensions and trailers are outside the supported subset and fail closed.
    pub fn step<'a>(&mut self, input: &'a [u8], limits: &Limits) -> Result<Chunk<'a>, ParseError> {
        match self.state {
            ChunkState::Size => {
                let Some((raw, consumed)) = line(input, limits.chunk_line, ParseError::ChunkLimit)?
                else {
                    return Ok(Chunk::NeedMore);
                };
                if raw.is_empty() || !raw.iter().all(u8::is_ascii_hexdigit) {
                    return Err(ParseError::ChunkSyntax);
                }
                let n = raw
                    .iter()
                    .try_fold(0usize, |n, b| {
                        n.checked_mul(16).and_then(|n| {
                            n.checked_add((*b as char).to_digit(16).unwrap_or(0) as usize)
                        })
                    })
                    .ok_or(ParseError::BodyLimit)?;
                self.total = self.total.checked_add(n).ok_or(ParseError::BodyLimit)?;
                if self.total > limits.body_bytes {
                    return Err(ParseError::BodyLimit);
                }
                self.chunks += 1;
                if self.chunks > limits.max_chunks {
                    return Err(ParseError::ChunkLimit);
                }
                self.state = if n == 0 {
                    ChunkState::FinalEnd
                } else {
                    ChunkState::Data(n)
                };
                Ok(Chunk::Progress(consumed))
            }
            ChunkState::Data(left) => {
                if input.is_empty() {
                    return Ok(Chunk::NeedMore);
                }
                let n = left.min(input.len());
                self.state = if n == left {
                    ChunkState::DataEnd
                } else {
                    ChunkState::Data(left - n)
                };
                Ok(Chunk::Data {
                    bytes: &input[..n],
                    consumed: n,
                })
            }
            ChunkState::DataEnd | ChunkState::FinalEnd => {
                if input.is_empty() {
                    return Ok(Chunk::NeedMore);
                }
                if input[0] != b'\r' {
                    return Err(if self.state == ChunkState::FinalEnd {
                        ParseError::TrailersUnsupported
                    } else {
                        ParseError::ChunkSyntax
                    });
                }
                if input.len() < 2 {
                    return Ok(Chunk::NeedMore);
                }
                if input[1] != b'\n' {
                    return Err(ParseError::ChunkSyntax);
                }
                if self.state == ChunkState::FinalEnd {
                    self.state = ChunkState::Done;
                    Ok(Chunk::End(2))
                } else {
                    self.state = ChunkState::Size;
                    Ok(Chunk::Progress(2))
                }
            }
            ChunkState::Done => Ok(Chunk::End(0)),
        }
    }
}
