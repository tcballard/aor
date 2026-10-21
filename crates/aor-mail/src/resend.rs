use crate::{Error, Message, Result, Transport, read_secret_file};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncWriteExt;
#[derive(Clone)]
pub struct Resend {
    api_key: Arc<String>,
    program: PathBuf,
}
impl Resend {
    pub fn from_key_file(path: &Path) -> Result<Self> {
        let bytes = read_secret_file(path, 512)?;
        let key = String::from_utf8(bytes)
            .map_err(|_| Error::Configuration)?
            .trim()
            .to_string();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Configuration);
        }
        Ok(Self {
            api_key: Arc::new(key),
            program: PathBuf::from("/usr/bin/curl"),
        })
    }
    fn config(&self, message: &Message) -> Result<String> {
        let body=serde_json::to_string(&serde_json::json!({"from":message.sender(),"to":[message.recipient()],"subject":message.subject(),"text":message.text()})).map_err(|_|Error::Configuration)?;
        let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        Ok(format!(
            "url = \"https://api.resend.com/emails\"\nrequest = \"POST\"\nproto = \"=https\"\nproto-redir = \"=https\"\nconnect-timeout = 5\nmax-time = 20\nsilent\noutput = \"/dev/null\"\nwrite-out = \"%{{http_code}}\"\nheader = \"Content-Type: application/json\"\nheader = \"Authorization: Bearer {}\"\nheader = \"Idempotency-Key: {}\"\ndata = \"{}\"\n",
            quote(&self.api_key),
            quote(message.idempotency_key()),
            quote(&body)
        ))
    }
}
impl Resend {
    async fn transmit(&self, config: String) -> Result<()> {
        let send = async {
            let mut child = tokio::process::Command::new(&self.program)
                .args(["--disable", "--config", "-"])
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| Error::Transport)?;
            let mut stdin = child.stdin.take().ok_or(Error::Transport)?;
            stdin
                .write_all(config.as_bytes())
                .await
                .map_err(|_| Error::Transport)?;
            stdin.shutdown().await.map_err(|_| Error::Transport)?;
            drop(stdin);
            let output = child
                .wait_with_output()
                .await
                .map_err(|_| Error::Transport)?;
            if !output.status.success()
                || output.stdout.len() != 3
                || !output.stdout.iter().all(u8::is_ascii_digit)
                || output.stdout[0] != b'2'
            {
                return Err(Error::Transport);
            }
            Ok(())
        };
        tokio::time::timeout(Duration::from_secs(25), send)
            .await
            .map_err(|_| Error::Transport)?
    }
}
impl Transport for Resend {
    async fn send(&self, message: &Message) -> Result<()> {
        self.transmit(self.config(message)?).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn real_curl_preserves_json_and_idempotency_headers_on_loopback() {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("loopback accept: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            assert_eq!(line, "POST /emails HTTP/1.1\r\n");
            let mut length = 0;
            let mut headers = String::new();
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                headers.push_str(&line);
            }
            assert!(length < 8192);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
            (headers, body)
        });
        let adapter = Resend {
            api_key: Arc::new("re_loopback_test".into()),
            program: "/usr/bin/curl".into(),
        };
        let message = Message {
            sender: "from@example.test".into(),
            recipient: "to@example.test".into(),
            subject: "Verify",
            text: "secret-code\n\"quoted\"".into(),
            key: "aor-mail/test".into(),
        };
        let config = adapter
            .config(&message)
            .unwrap()
            .replace(
                "https://api.resend.com/emails",
                &format!("http://{address}/emails"),
            )
            .replace("=https", "=http");
        adapter.transmit(config).await.unwrap();
        let (headers, body) = server.join().unwrap();
        assert!(headers.contains("Authorization: Bearer re_loopback_test"));
        assert!(headers.contains("Idempotency-Key: aor-mail/test"));
        let data: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(data["text"], message.text());
        assert_eq!(data["to"][0], message.recipient());
    }
    #[tokio::test]
    async fn credentials_use_stdin_and_provider_failures_are_redacted() {
        let dir = std::env::temp_dir().join(format!("aor-curl-{}", aor_db::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let program = dir.join("curl");
        let config = dir.join("config");
        let script = format!(
            "#!/bin/sh\n[ \"$#\" = 3 ] && [ \"$1\" = --disable ] && [ \"$2\" = --config ] && [ \"$3\" = - ] || exit 1\n/bin/cat > '{}'\nprintf 200\n",
            config.display()
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adapter = Resend {
            api_key: Arc::new("re_private_test".into()),
            program: program.clone(),
        };
        let message = Message {
            sender: "from@example.test".into(),
            recipient: "to@example.test".into(),
            subject: "Verify",
            text: "secret-code\n\"quoted\"".into(),
            key: "aor-mail/test".into(),
        };
        adapter.send(&message).await.unwrap();
        let bytes = std::fs::read_to_string(&config).unwrap();
        assert!(bytes.contains("Authorization: Bearer re_private_test"));
        assert!(bytes.contains("Idempotency-Key: aor-mail/test"));
        assert!(!bytes.contains("secret-code\n"));
        assert!(!bytes.contains("location"));
        assert!(bytes.contains("proto = \"=https\""));
        for status in ["409", "429", "500", "302"] {
            std::fs::write(
                &program,
                format!("#!/bin/sh\n/bin/cat >/dev/null\nprintf {status}\n"),
            )
            .unwrap();
            assert_eq!(
                adapter.send(&message).await.unwrap_err().to_string(),
                "AOR-MAIL-TRANSPORT"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
