use aor_http::{Listener, Response};
use std::{collections::BTreeMap, io, path::Path, sync::Arc};
struct Asset {
    bytes: Vec<u8>,
    mime: &'static str,
}
fn load(root: &Path) -> io::Result<BTreeMap<String, Asset>> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, Asset>,
        total: &mut usize,
    ) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "symlinks are not allowed in the archive",
                ));
            }
            if kind.is_dir() {
                walk(root, &entry.path(), out, total)?;
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            let mime = match path.extension().and_then(|x| x.to_str()).unwrap_or("") {
                "html" => "text/html; charset=utf-8",
                "css" => "text/css; charset=utf-8",
                "js" => "text/javascript; charset=utf-8",
                "xml" => "application/xml",
                "txt" => "text/plain; charset=utf-8",
                _ => continue,
            };
            if entry.metadata()?.len() > 8 * 1024 * 1024 || out.len() >= 1024 {
                return Err(io::Error::other("archive snapshot limit exceeded"));
            }
            let bytes = std::fs::read(&path)?;
            *total += bytes.len();
            if *total > 64 * 1024 * 1024 {
                return Err(io::Error::other("archive snapshot limit exceeded"));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_str()
                .ok_or_else(|| io::Error::other("non-UTF8 archive path"))?
                .to_owned();
            out.insert(format!("/{relative}"), Asset { bytes, mime });
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out, &mut 0)?;
    Ok(out)
}

use aor_router::{AppError, Context, Route, Router, route};
use aor_tmpl::{Template, TemplateContext, theme::Theme};
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(TemplateContext)]
struct Page {
    title: String,
    dev: bool,
}
fn response(status: u16, bytes: Vec<u8>, mime: &str) -> Response {
    Response::new(status, bytes)
        .header("Content-Type", mime)
        .unwrap()
}
async fn health(_: Context) -> Result<Response, AppError> {
    Ok(response(
        200,
        b"{\"status\":\"ok\",\"public_ready\":false}".to_vec(),
        "application/json",
    ))
}
#[tokio::main]
async fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut address = "127.0.0.1:3000".to_owned();
    let mut socket = None;
    let mut root = None;
    let mut dev = false;
    let mut list = false;
    let mut json = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "serve" => {}
            "routes" => list = true,
            "--json" => json = true,
            "--dev" => dev = true,
            "--listen" => {
                address = args
                    .next()
                    .ok_or_else(|| io::Error::other("--listen requires an address"))?
            }
            "--socket" => {
                socket = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--socket requires a path"))?,
                )
            }
            "--root" => {
                root = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--root requires a directory"))?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "aor-archive serve [--listen 127.0.0.1:3000 | --socket PATH] [--root DIRECTORY] [--dev]\naor-archive routes [--json]\nDevelopment only. No public-use clearance."
                );
                return Ok(());
            }
            _ => return Err(io::Error::other(format!("unknown argument: {arg}"))),
        }
    }
    let custom = root.is_some();
    let assets = Arc::new(if let Some(root) = root {
        load(Path::new(&root))?
    } else {
        BTreeMap::from([(
            "/archive.css".to_owned(),
            Asset {
                bytes: include_bytes!("../public/archive.css").to_vec(),
                mime: "text/css; charset=utf-8",
            },
        )])
    });
    let generation = Arc::new(AtomicU64::new(1));
    let mut routes = vec![route!(GET "/healthz"=>health)];
    if !custom {
        let home = move |_: Context| async move {
            let page = Page {
                title: "A small site. A whole framework underneath.".into(),
                dev,
            };
            let rendered = if dev {
                Template::render_file(Path::new("apps/archive/templates/index.html"), &page)
            } else {
                Template::parse(include_str!("../templates/index.html"))
                    .and_then(|t| t.render(&page))
            };
            match rendered {
                Ok(html) => Ok(response(200, html.into_bytes(), "text/html; charset=utf-8")),
                Err(e) if dev => Ok(response(
                    500,
                    format!(
                        "<h1>Template error</h1><pre>{}</pre>",
                        aor_tmpl::escape(&e.to_string())
                    )
                    .into_bytes(),
                    "text/html; charset=utf-8",
                )),
                Err(_) => Err(AppError::Internal),
            }
        };
        routes.push(route!(GET "/"=>home));
    }
    for path in assets.keys() {
        let assets = assets.clone();
        let key = path.clone();
        let handler = move |_: Context| {
            let assets = assets.clone();
            let key = key.clone();
            async move {
                let a = &assets[&key];
                let bytes = if dev && key == "/archive.css" && !custom {
                    std::fs::read("apps/archive/public/archive.css")
                        .map_err(|_| AppError::Internal)?
                } else {
                    a.bytes.clone()
                };
                Ok(response(200, bytes, a.mime))
            }
        };
        let path = if path == "/index.html" { "/" } else { path };
        routes.push(Route::public("GET", path, "archive::asset", handler));
    }
    let theme = |_: Context| async {
        Ok(response(
            200,
            Theme::active().css().into_bytes(),
            "text/css; charset=utf-8",
        )
        .header("Cache-Control", "no-store")
        .unwrap())
    };
    routes.push(route!(GET "/_aor/theme.css"=>theme));
    if dev {
        let counter = generation.clone();
        let reload = move |_: Context| {
            let counter = counter.clone();
            async move {
                let (tx, rx) = tokio::sync::mpsc::channel(2);
                tokio::spawn(async move {
                    loop {
                        let data = format!("data: {}\n\n", counter.load(Ordering::Relaxed));
                        if tx.send(Ok(data.into_bytes())).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    }
                });
                Ok(Response::stream(200, rx)
                    .header("Content-Type", "text/event-stream")
                    .unwrap()
                    .header("Cache-Control", "no-store")
                    .unwrap())
            }
        };
        routes.push(route!(GET "/_aor/reload"=>reload));
        let javascript = |_: Context| async {
            Ok(response(200,br#"let version; const source=new EventSource('/_aor/reload'); source.onmessage=(event)=>{if(version!==undefined&&version!==event.data)location.reload();version=event.data;}; setInterval(async()=>{try{const text=await(await fetch('/_aor/dev-status')).text();const el=document.getElementById('dev-status');if(el)el.textContent=text;}catch{}},1000);"#.to_vec(),"text/javascript; charset=utf-8"))
        };
        routes.push(route!(GET "/_aor/reload.js"=>javascript));
        let status = |_: Context| async {
            Ok(response(
                200,
                std::fs::read(".aor/dev-status").unwrap_or_else(|_| b"current".to_vec()),
                "text/plain; charset=utf-8",
            )
            .header("Cache-Control", "no-store")
            .unwrap())
        };
        routes.push(route!(GET "/_aor/dev-status"=>status));
    }
    let router = Arc::new(Router::new(routes).map_err(io::Error::other)?);
    if list {
        if json {
            println!("{}", serde_json::to_string_pretty(&router.routes())?);
        } else {
            for r in router.routes() {
                println!("{} {} {} {}", r.method, r.path, r.handler, r.authentication);
            }
        }
        return Ok(());
    }
    let watcher = if dev {
        let mut roots = vec![
            "apps/archive/templates".into(),
            "apps/archive/public".into(),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let current = Path::new(&home).join(".config/omarchy/current");
            roots.push(current.clone());
            if let Ok(active) = std::fs::canonicalize(current.join("theme")) {
                roots.push(active);
            }
        }
        let mut watcher = aor_dev::Watcher::new(roots, std::time::Duration::from_millis(80))?;
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(40));
            let mut last_theme = Theme::active();
            loop {
                interval.tick().await;
                match watcher.poll() {
                    Ok(changes) => {
                        if !changes.is_empty() {
                            generation.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        eprintln!("AOR_DEV_WATCH: {e}");
                        break;
                    }
                }
                let theme = Theme::active();
                if theme != last_theme {
                    last_theme = theme;
                    generation.fetch_add(1, Ordering::Relaxed);
                }
            }
        }))
    } else {
        None
    };
    let listener = if let Some(path) = socket {
        Listener::unix(path)?
    } else {
        Listener::tcp(address.parse().map_err(io::Error::other)?).await?
    };
    eprintln!("AoR archive ready (development only)");
    let result = aor_http::serve(
        listener,
        aor_http::Limits::default(),
        move |r| {
            let router = router.clone();
            async move { router.handle(r).await }
        },
        async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
        },
    )
    .await;
    if let Some(w) = watcher {
        w.abort();
    }
    result
}
