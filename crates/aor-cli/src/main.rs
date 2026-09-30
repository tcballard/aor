use clap::{Parser, Subcommand};
use std::{io, path::PathBuf, process::Command};
#[derive(Parser)]
#[command(
    name = "cargo aor",
    version,
    about = "AoR development tooling; incomplete v0.3 implementation"
)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Checks foundation integrity. Public readiness fails until all gates are evidenced.
    Verify {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        development: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Read-only diagnostics for the current workspace and toolchain.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Prints the archive application's actual registered routes.
    Routes {
        #[arg(long)]
        json: bool,
    },
    /// Builds and watches the archive; template/theme changes reload without a rebuild.
    Dev,
}
fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("AOR_CLI_ERROR: {e}");
        std::process::exit(2);
    }
}
async fn run() -> io::Result<()> {
    let mut raw: Vec<_> = std::env::args_os().collect();
    if raw.get(1).is_some_and(|a| a == "aor") {
        raw.remove(1);
    }
    let args = Args::parse_from(raw);
    match args.command {
        Action::Verify {
            json,
            development,
            root,
        } => {
            let report = aor_verify::verify(&root)?;
            let pass = if development {
                report.development_pass
            } else {
                report.public_ready
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "AoR foundation verification — public_ready={}",
                    report.public_ready
                );
                for f in report.findings {
                    println!("{} {}: {}", f.rule_id, f.status, f.detail);
                }
            }
            if !pass {
                std::process::exit(1);
            }
        }
        Action::Doctor { json } => {
            let mut checks = std::collections::BTreeMap::new();
            for name in ["rustc", "cargo"] {
                let result = Command::new(name).arg("--version").output();
                let value = result
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                    .unwrap_or_else(|| "missing".into());
                checks.insert(name, value);
            }
            checks.insert("os", std::env::consts::OS.to_owned());
            checks.insert("database", "not implemented at this milestone".into());
            checks.insert("public_ready", "false".into());
            let ok = checks["rustc"] != "missing"
                && checks["cargo"] != "missing"
                && checks["os"] == "linux";
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"schema_version":1,"development_ready":ok,"checks":checks})
                    )?
                );
            } else {
                for (k, v) in checks {
                    println!("{k}: {v}");
                }
            }
            if !ok {
                std::process::exit(1);
            }
        }
        Action::Routes { json } => {
            let mut cmd = Command::new(cargo());
            cmd.args([
                "run",
                "--quiet",
                "--locked",
                "-p",
                "aor-archive",
                "--",
                "routes",
            ]);
            if json {
                cmd.arg("--json");
            }
            let status = cmd.status()?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Action::Dev => aor_dev::run_archive().await?,
    }
    Ok(())
}
