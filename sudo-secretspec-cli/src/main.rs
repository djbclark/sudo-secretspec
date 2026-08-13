use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "sudo-secretspec",
    version,
    about = "Privilege-separated SecretSpec companion"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    Add {
        name: String,
        #[arg(long)]
        reason: String,
    },
    Set {
        name: String,
        #[arg(long)]
        reason: String,
    },
    Delete {
        name: String,
        #[arg(long)]
        reason: String,
    },
    Get {
        name: String,
        #[arg(long)]
        reason: String,
    },
    Check {
        #[arg(long)]
        reason: String,
    },
    Export {
        #[arg(long)]
        reason: String,
    },
    Run {
        #[arg(long)]
        reason: String,
        #[arg(last = true, required = true)]
        command: Vec<OsString>,
    },
    Install {
        #[arg(long)]
        declarations: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        adopt_existing: bool,
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        service_user: Option<String>,
        #[arg(long)]
        service_group: Option<String>,
        #[arg(long)]
        operator: Option<String>,
    },
    Doctor {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Rollback {
        snapshot: PathBuf,
    },
    #[command(hide = true, name = "__broker")]
    Broker {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Add { name, reason } => lifecycle("add", &name, &reason),
        Cmd::Set { name, reason } => lifecycle("set", &name, &reason),
        Cmd::Delete { name, reason } => lifecycle("delete", &name, &reason),
        Cmd::Get { name, reason } => lifecycle("get", &name, &reason),
        Cmd::Check { reason } => lifecycle("check", "", &reason),
        Cmd::Export { reason } => lifecycle("export", "", &reason),
        Cmd::Run { reason, command } => run_target(&reason, &command),
        Cmd::Install {
            declarations,
            dry_run,
            adopt_existing,
            vault,
            service_user,
            service_group,
            operator,
        } => {
            let mut req = sudo_secretspec_cli::install::InstallRequest::from_cli(
                declarations,
                dry_run,
                adopt_existing,
            );
            if let Some(v) = vault {
                req.vault = v;
            }
            if let Some(u) = service_user {
                req.service_user = u;
            }
            if let Some(g) = service_group {
                req.service_group = g;
            }
            if let Some(o) = operator {
                req.operator = o;
            }
            // Install itself needs interactive sudo/Touch ID, not NOPASSWD -n.
            if unsafe { libc::geteuid() } != 0 {
                let status = Command::new("sudo")
                    .arg(self_exe())
                    .arg("install")
                    .arg("--declarations")
                    .arg(&req.declarations)
                    .args(if req.dry_run {
                        vec!["--dry-run"]
                    } else {
                        vec![]
                    })
                    .args(if req.adopt_existing {
                        vec!["--adopt-existing"]
                    } else {
                        vec![]
                    })
                    .arg("--vault")
                    .arg(&req.vault)
                    .arg("--service-user")
                    .arg(&req.service_user)
                    .arg("--service-group")
                    .arg(&req.service_group)
                    .arg("--operator")
                    .arg(&req.operator)
                    .status();
                match status {
                    Ok(s) if s.success() => return,
                    Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                    Err(e) => {
                        eprintln!("cannot elevate install: {e}");
                        std::process::exit(2);
                    }
                }
            }
            if let Err(e) = sudo_secretspec_cli::install::run(req) {
                eprintln!("install denied: {e}");
                std::process::exit(2);
            }
        }
        Cmd::Doctor { json, config } => doctor(json, config),
        Cmd::Rollback { snapshot } => {
            if unsafe { libc::geteuid() } != 0 {
                let status = Command::new("sudo")
                    .arg(self_exe())
                    .arg("rollback")
                    .arg(&snapshot)
                    .status();
                match status {
                    Ok(s) if s.success() => return,
                    Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                    Err(e) => {
                        eprintln!("cannot elevate rollback: {e}");
                        std::process::exit(2);
                    }
                }
            }
            if let Err(e) = sudo_secretspec_cli::rollback::run(&snapshot) {
                eprintln!("rollback denied: {e}");
                std::process::exit(2);
            }
        }
        Cmd::Broker { args } => {
            if let Err(code) = sudo_secretspec_cli::broker::dispatch(&args) {
                std::process::exit(code);
            }
        }
    }
}

fn self_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sudo-secretspec"))
}

fn privileged_broker() -> PathBuf {
    let candidate = PathBuf::from("/usr/local/libexec/sudo-secretspec");
    if candidate.is_file() {
        candidate
    } else {
        self_exe()
    }
}

fn lifecycle(op: &str, name: &str, reason: &str) {
    let mut cmd = Command::new("sudo");
    cmd.arg("-n")
        .arg(privileged_broker())
        .arg("__broker")
        .arg(format!("source-{op}"))
        .arg("--client")
        .arg(detect_client())
        .arg("--reason")
        .arg(reason);
    if !name.is_empty() {
        cmd.arg("--name").arg(name);
    }
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("cannot invoke broker: {e}");
        std::process::exit(2);
    });
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn detect_client() -> &'static str {
    for (key, family) in [
        ("HERMES_SESSION_ID", "hermes"),
        ("CLAUDE_SESSION_ID", "claude"),
        ("CODEX_THREAD_ID", "codex"),
        ("OPENCODE_SESSION_ID", "opencode"),
        ("CURSOR_SESSION_ID", "cursor"),
        ("AGY_SESSION_ID", "agy"),
    ] {
        if std::env::var_os(key).is_some() {
            return family;
        }
    }
    "unknown"
}

fn run_target(reason: &str, command: &[OsString]) {
    let sep = command
        .iter()
        .position(|a| a == "--")
        .unwrap_or(command.len());
    let (pre, target) = command.split_at(sep);
    let target = if target.is_empty() { pre } else { &target[1..] };
    if target.is_empty() {
        eprintln!("run requires a command after --");
        std::process::exit(2);
    }

    let output = Command::new("sudo")
        .arg("-n")
        .arg(privileged_broker())
        .arg("__broker")
        .arg("source-export")
        .arg("--client")
        .arg(detect_client())
        .arg("--reason")
        .arg(reason)
        .arg("--command-basename")
        .arg(target[0].to_str().unwrap_or("unknown"))
        .output()
        .unwrap_or_else(|e| {
            eprintln!("cannot invoke broker: {e}");
            std::process::exit(2);
        });

    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let env: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        eprintln!("broker returned invalid JSON: {e}");
        std::process::exit(2);
    });

    let mut cmd = Command::new(&target[0]);
    cmd.args(&target[1..]);
    for (key, _) in std::env::vars() {
        if key.starts_with("SECRETSPEC_") {
            cmd.env_remove(&key);
        }
    }
    if let Some(obj) = env.as_object() {
        for (key, value) in obj {
            if let Some(v) = value.as_str() {
                cmd.env(key, v);
            }
        }
    }

    let err = cmd.exec();
    eprintln!("cannot exec target: {err}");
    std::process::exit(127);
}

fn doctor(json: bool, config: Option<PathBuf>) {
    let config = config.unwrap_or_else(|| PathBuf::from("/usr/local/etc/sudo-secretspec.toml"));
    let layout = match sudo_secretspec_cli::load_config(&config) {
        Ok(l) => l,
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"ok": false, "error": format!("{e}")})
                );
            } else {
                eprintln!("doctor: config error: {e}");
            }
            std::process::exit(1);
        }
    };

    let report = sudo_secretspec_cli::inspect(&layout);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else if report.ok {
        println!("doctor: OK");
    } else {
        eprintln!("doctor: FAILED");
        for finding in &report.findings {
            match &finding.path {
                Some(path) => eprintln!("- {} ({}): {}", finding.code, path, finding.detail),
                None => eprintln!("- {}: {}", finding.code, finding.detail),
            }
        }
        std::process::exit(1);
    }
}
