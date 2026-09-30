//! Linux inotify watcher with debounce and directory reconciliation.
use nix::{
    errno::Errno,
    sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Template,
    Asset,
    Rust,
    Migration,
    Other,
}
struct Watch {
    descriptor: WatchDescriptor,
    inode: u64,
}
pub struct Watcher {
    inotify: Inotify,
    roots: Vec<PathBuf>,
    watches: BTreeMap<PathBuf, Watch>,
    pending: BTreeSet<Change>,
    last_event: Option<Instant>,
    last_scan: Instant,
    debounce: Duration,
}
impl Watcher {
    pub fn new(roots: Vec<PathBuf>, debounce: Duration) -> io::Result<Self> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
            .map_err(io::Error::from)?;
        let mut this = Self {
            inotify,
            roots,
            watches: BTreeMap::new(),
            pending: BTreeSet::new(),
            last_event: None,
            last_scan: Instant::now(),
            debounce,
        };
        this.reconcile()?;
        Ok(this)
    }
    fn reconcile(&mut self) -> io::Result<()> {
        fn directories(path: &Path, out: &mut BTreeMap<PathBuf, u64>) -> io::Result<()> {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            if metadata.is_file() {
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                out.insert(parent.to_owned(), std::fs::metadata(parent)?.ino());
                return Ok(());
            }
            if metadata.is_symlink() || !metadata.is_dir() {
                return Ok(());
            }
            if out.len() >= 4096 {
                return Err(io::Error::other("watch directory limit exceeded"));
            }
            out.insert(path.to_path_buf(), metadata.ino());
            for e in std::fs::read_dir(path)? {
                let e = e?;
                if e.file_type()?.is_dir() {
                    directories(&e.path(), out)?;
                }
            }
            Ok(())
        }
        let mut wanted = BTreeMap::new();
        for root in &self.roots {
            directories(root, &mut wanted)?;
        }
        let remove: Vec<_> = self
            .watches
            .iter()
            .filter(|(p, w)| wanted.get(*p) != Some(&w.inode))
            .map(|(p, _)| p.clone())
            .collect();
        for p in remove {
            if let Some(w) = self.watches.remove(&p) {
                let _ = self.inotify.rm_watch(w.descriptor);
                self.pending.insert(classify(&p));
                self.last_event = Some(Instant::now());
            }
        }
        for (path, inode) in wanted {
            if !self.watches.contains_key(&path) {
                let descriptor = self
                    .inotify
                    .add_watch(
                        &path,
                        AddWatchFlags::IN_CLOSE_WRITE
                            | AddWatchFlags::IN_CREATE
                            | AddWatchFlags::IN_DELETE
                            | AddWatchFlags::IN_MOVED_FROM
                            | AddWatchFlags::IN_MOVED_TO
                            | AddWatchFlags::IN_DELETE_SELF
                            | AddWatchFlags::IN_MOVE_SELF
                            | AddWatchFlags::IN_ATTRIB,
                    )
                    .map_err(io::Error::from)?;
                self.watches.insert(path, Watch { descriptor, inode });
            }
        }
        self.last_scan = Instant::now();
        Ok(())
    }
    pub fn poll(&mut self) -> io::Result<BTreeSet<Change>> {
        loop {
            match self.inotify.read_events() {
                Ok(events) => {
                    if events.is_empty() {
                        break;
                    }
                    for event in events {
                        if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                            self.pending.extend([
                                Change::Rust,
                                Change::Template,
                                Change::Asset,
                                Change::Migration,
                            ]);
                            self.last_event = Some(Instant::now());
                            continue;
                        }
                        let path = self
                            .watches
                            .iter()
                            .find(|(_, w)| w.descriptor == event.wd)
                            .map(|(p, _)| p.clone());
                        if let Some(mut path) = path {
                            if let Some(name) = event.name {
                                let s = name.to_string_lossy();
                                if s.starts_with('.') || s.ends_with('~') || s.ends_with(".swp") {
                                    continue;
                                }
                                path.push(name);
                            }
                            self.pending.insert(classify(&path));
                            self.last_event = Some(Instant::now());
                        }
                    }
                }
                Err(Errno::EAGAIN) => break,
                Err(e) => return Err(io::Error::from(e)),
            }
        }
        if self.last_scan.elapsed() >= Duration::from_millis(200) {
            self.reconcile()?;
        }
        if self
            .last_event
            .is_some_and(|t| t.elapsed() >= self.debounce)
        {
            self.last_event = None;
            return Ok(std::mem::take(&mut self.pending));
        }
        Ok(BTreeSet::new())
    }
}
fn classify(path: &Path) -> Change {
    if path.extension().is_some_and(|e| e == "rs")
        || path
            .file_name()
            .is_some_and(|n| n == "Cargo.toml" || n == "Cargo.lock" || n == "config.toml")
    {
        return Change::Rust;
    }
    if path.components().any(|p| p.as_os_str() == "migrations") {
        Change::Migration
    } else if path.components().any(|p| p.as_os_str() == "templates") {
        Change::Template
    } else if path
        .components()
        .any(|p| p.as_os_str() == "assets" || p.as_os_str() == "public")
    {
        Change::Asset
    } else {
        Change::Other
    }
}
/// Framework development command. Compiles before replacing the last-good process.
pub async fn run_archive() -> io::Result<()> {
    use tokio::process::Command;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let meta = Command::new(&cargo)
        .args(["metadata", "--no-deps", "--format-version=1"])
        .output()
        .await?;
    if !meta.status.success() {
        return Err(io::Error::other("cargo metadata failed"));
    }
    let meta: serde_json::Value = serde_json::from_slice(&meta.stdout)?;
    let binary = Path::new(
        meta["target_directory"]
            .as_str()
            .ok_or_else(|| io::Error::other("missing cargo target directory"))?,
    )
    .join("debug/aor-archive");
    if !Command::new(&cargo)
        .args(["build", "--locked", "-p", "aor-archive"])
        .status()
        .await?
        .success()
    {
        return Err(io::Error::other("initial build failed"));
    }
    std::fs::create_dir_all(".aor")?;
    std::fs::write(".aor/dev-status", "current")?;
    let spawn = || {
        let mut c = Command::new(&binary);
        c.args(["serve", "--dev"]);
        c.kill_on_drop(true);
        c.spawn()
    };
    let mut child = spawn()?;
    let mut watcher = Watcher::new(
        vec![
            "crates".into(),
            "apps".into(),
            "Cargo.toml".into(),
            "Cargo.lock".into(),
            ".cargo".into(),
        ],
        Duration::from_millis(150),
    )?;
    let mut interval = tokio::time::interval(Duration::from_millis(50));
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {_ = tokio::signal::ctrl_c()=>break,_=terminate.recv()=>break,_=interval.tick()=>{}}
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!("archive exited: {status}")));
        }
        let changes = watcher.poll()?;
        if changes.contains(&Change::Rust) || changes.contains(&Change::Migration) {
            std::fs::write(
                ".aor/dev-status",
                "building; last-good server remains active",
            )?;
            let status = Command::new(&cargo)
                .args(["build", "--locked", "-p", "aor-archive"])
                .status()
                .await?;
            if status.success() {
                stop_child(&mut child).await?;
                child = spawn()?;
                std::fs::write(".aor/dev-status", "current")?;
            } else {
                std::fs::write(
                    ".aor/dev-status",
                    "stale: build failed; serving last-good binary",
                )?;
                eprintln!("AOR_DEV_STALE: build failed; last-good process retained");
            }
        }
    }
    stop_child(&mut child).await
}
async fn stop_child(child: &mut tokio::process::Child) -> io::Result<()> {
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    if tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .is_err()
    {
        child.kill().await?;
        child.wait().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_rename_and_recreate_are_observed() {
        let root = std::env::temp_dir().join(format!("aor-watch-{}", std::process::id()));
        let dir = root.join("templates");
        std::fs::create_dir_all(&dir).unwrap();
        let mut watcher = Watcher::new(vec![dir.clone()], Duration::from_millis(10)).unwrap();
        std::fs::write(dir.join("page.html"), "one").unwrap();
        let mut seen = BTreeSet::new();
        for _ in 0..20 {
            seen.extend(watcher.poll().unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen.contains(&Change::Template));
        std::fs::remove_dir_all(&dir).unwrap();
        std::thread::sleep(Duration::from_millis(220));
        let _ = watcher.poll().unwrap();
        std::fs::create_dir(&dir).unwrap();
        std::thread::sleep(Duration::from_millis(220));
        let _ = watcher.poll().unwrap();
        std::fs::write(dir.join(".page.tmp"), "two").unwrap();
        std::fs::rename(dir.join(".page.tmp"), dir.join("page.html")).unwrap();
        let mut seen = BTreeSet::new();
        for _ in 0..20 {
            seen.extend(watcher.poll().unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen.contains(&Change::Template));
        std::fs::remove_dir_all(root).unwrap();
    }
}
