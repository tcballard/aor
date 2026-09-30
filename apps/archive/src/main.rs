mod store;
use aor_http::{Listener, Response};
use std::{collections::BTreeMap, io, path::Path, sync::Arc};
const CSS_PATH: &str = include_str!(concat!(env!("OUT_DIR"), "/archive_css_path.txt"));
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
    database: bool,
    editions: Vec<store::Summary>,
}
fn response(status: u16, bytes: Vec<u8>, mime: &str) -> Response {
    Response::new(status, bytes)
        .header("Content-Type", mime)
        .unwrap()
}
#[aor_router::handler]
async fn health(_: aor_router::RequestId) -> Result<Response, AppError> {
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
    let mut local = None;
    let mut migrate = false;
    let mut import = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "serve" => {}
            "migrate" => migrate = true,
            "import" => {
                import = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("import requires a JSON file"))?,
                )
            }
            "--sqlite" => {
                local = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--sqlite requires a file"))?,
                )
            }
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
                    "aor-archive serve [--listen 127.0.0.1:3000 | --socket PATH] [--root DIRECTORY] [--dev]\naor-archive routes [--json]\naor-archive migrate [--sqlite FILE]\naor-archive import FILE [--sqlite FILE]\nSet AOR_DATABASE_URL for PostgreSQL or use --sqlite FILE with serve.\nDevelopment only. No public-use clearance."
                );
                return Ok(());
            }
            _ => return Err(io::Error::other(format!("unknown argument: {arg}"))),
        }
    }
    let store = if list {
        None
    } else {
        store::Store::open(local.as_deref())?
    };
    if migrate || import.is_some() {
        let db = store
            .as_ref()
            .ok_or_else(|| io::Error::other("set AOR_DATABASE_URL or --sqlite FILE"))?;
        if migrate {
            println!("Applied {} migrations", db.migrate().await?);
        }
        if let Some(file) = import {
            println!("Imported {} editions", db.import(Path::new(&file)).await?);
        }
        return Ok(());
    }
    let custom = root.is_some();
    let assets = Arc::new(if let Some(root) = root {
        load(Path::new(&root))?
    } else {
        ["/archive.css", CSS_PATH]
            .into_iter()
            .map(|path| {
                (
                    path.to_owned(),
                    Asset {
                        bytes: include_bytes!("../public/archive.css").to_vec(),
                        mime: "text/css; charset=utf-8",
                    },
                )
            })
            .collect()
    });
    let generation = Arc::new(AtomicU64::new(1));
    let mut routes = vec![route!(GET "/healthz"=>health)];
    if !custom {
        let database = store.clone();
        let home = move |_: Context| {
            let database = database.clone();
            async move {
                let counter = aor_db::QueryCounter::default();
                let editions = if let Some(db) = &database {
                    counter
                        .scope(db.list())
                        .await
                        .map_err(|_| AppError::Internal)?
                } else {
                    Vec::new()
                };
                if dev {
                    for (_, count) in counter.repeated(3) {
                        eprintln!("possible N+1: {count} repeated queries in archive request");
                    }
                }
                let page = Page {
                    title: "A small site. A whole framework underneath.".into(),
                    dev,
                    database: database.is_some(),
                    editions,
                };
                let rendered = if dev {
                    Template::render_file(Path::new("apps/archive/templates/index.html"), &page)
                } else {
                    Template::parse(include_str!("../templates/index.html"))
                        .and_then(|t| t.render(&page))
                };
                match rendered {
                    Ok(html) => {
                        let html = if dev {
                            html
                        } else {
                            html.replace("/archive.css", CSS_PATH)
                        };
                        Ok(response(200, html.into_bytes(), "text/html; charset=utf-8"))
                    }
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
            }
        };
        routes.push(route!(GET "/"=>home));
        if let Some(database) = store.clone() {
            let edition = move |ctx: Context| {
                let database = database.clone();
                async move {
                    let slug = ctx.path.get::<aor_router::Slug>("slug")?.to_string();
                    let page = database
                        .edition(slug)
                        .await
                        .map_err(|_| AppError::Internal)?
                        .ok_or(AppError::NotFound)?;
                    let template = if dev {
                        std::fs::read_to_string("apps/archive/templates/edition.html")
                            .map_err(|_| AppError::Internal)?
                    } else {
                        include_str!("../templates/edition.html").into()
                    };
                    let html = Template::parse(&template)
                        .and_then(|t| t.render(&page))
                        .map_err(|_| AppError::Internal)?;
                    let html = if dev {
                        html
                    } else {
                        html.replace("/archive.css", CSS_PATH)
                    };
                    Ok(response(200, html.into_bytes(), "text/html; charset=utf-8"))
                }
            };
            routes.push(route!(GET "/editions/{slug}"=>edition));
        }
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
                Ok(response(200, bytes, a.mime)
                    .header(
                        "Cache-Control",
                        if key == CSS_PATH {
                            "public, max-age=31536000, immutable"
                        } else {
                            "no-cache"
                        },
                    )
                    .unwrap())
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
    let router = Arc::new(Router::new(routes).map_err(io::Error::other)?.tracing(dev));
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
