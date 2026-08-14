use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use clap::{Parser, Subcommand};

/// Absolute path to sudo. Never resolve this through `PATH`: the caller
/// controls the environment, and a planted `sudo` earlier in `PATH` would
/// silently satisfy every broker call with forged values and no audit event.
const SUDO: &str = "/usr/bin/sudo";

/// Installed path of the privileged broker. The sudoers policy grants
/// NOPASSWD execution of this path, so when the running binary *is* this
/// path it may only serve the operations that policy is meant to expose.
const BROKER_PATH: &str = "/usr/local/libexec/sudo-secretspec";

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
    /// Install or adopt the privileged boundary.
    ///
    /// Typical short form:
    ///   sudo-secretspec install
    ///   sudo-secretspec install --adopt-existing
    Install {
        /// Declaration TOML. Optional: auto-detected or prompted.
        #[arg(long)]
        declarations: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        /// Adopt an existing vault/service identity instead of creating one.
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
        /// Never prompt; fail if required values cannot be defaulted.
        #[arg(long)]
        non_interactive: bool,
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

/// True when this process is the installed privileged broker, i.e. it was
/// reached through the NOPASSWD sudoers rule rather than the public client.
fn invoked_as_privileged_broker() -> bool {
    let broker = std::fs::canonicalize(BROKER_PATH);
    match (std::env::current_exe(), broker) {
        (Ok(me), Ok(broker)) => std::fs::canonicalize(me)
            .map(|me| me == broker)
            .unwrap_or(false),
        _ => false,
    }
}

fn main() {
    let cli = Cli::parse();

    // The libexec path is NOPASSWD for the operator. Boundary lifecycle must
    // stay behind an interactive authentication, so refuse anything other than
    // the mediated operations when we were invoked through that path.
    if invoked_as_privileged_broker() && !matches!(cli.cmd, Cmd::Broker { .. } | Cmd::Doctor { .. })
    {
        eprintln!(
            "sudo-secretspec: the privileged broker path serves only __broker and doctor;\n\
             run install/rollback through /usr/local/bin/sudo-secretspec so they require\n\
             interactive authentication"
        );
        std::process::exit(2);
    }

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
            non_interactive,
        } => run_install(
            declarations,
            dry_run,
            adopt_existing,
            vault,
            service_user,
            service_group,
            operator,
            non_interactive,
        ),
        Cmd::Doctor { json, config } => doctor(json, config),
        Cmd::Rollback { snapshot } => {
            if unsafe { libc::geteuid() } != 0 {
                let status = Command::new(SUDO)
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
    let candidate = PathBuf::from(BROKER_PATH);
    if candidate.is_file() {
        candidate
    } else {
        self_exe()
    }
}

fn is_tty() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn prompt_line(label: &str, default: Option<&str>) -> Option<String> {
    if !is_tty() {
        return default.map(str::to_string);
    }
    let mut stdout = io::stdout();
    match default {
        Some(d) if !d.is_empty() => {
            let _ = write!(stdout, "{label} [{d}]: ");
        }
        _ => {
            let _ = write!(stdout, "{label}: ");
        }
    }
    let _ = stdout.flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default.map(str::to_string);
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        default.map(str::to_string)
    } else {
        Some(trimmed.to_string())
    }
}

fn prompt_yes_no(label: &str, default_yes: bool) -> bool {
    if !is_tty() {
        return default_yes;
    }
    let def = if default_yes { "Y/n" } else { "y/N" };
    let mut stdout = io::stdout();
    let _ = write!(stdout, "{label} [{def}]: ");
    let _ = stdout.flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default_yes;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}

fn declaration_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(env) = std::env::var("SUDO_SECRETSPEC_DECLARATIONS") {
        out.push(PathBuf::from(env));
    }
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join("ops/site-private/secretspec.toml.example"));
    }
    out.push(PathBuf::from(
        "/usr/local/share/sudo-secretspec/secretspec.toml",
    ));
    // Common worktree layout relative to cwd.
    out.push(PathBuf::from("site-private/secretspec.toml.example"));
    out.push(PathBuf::from("../site-private/secretspec.toml.example"));
    out
}

fn resolve_declarations(
    explicit: Option<PathBuf>,
    non_interactive: bool,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if path.is_file() && !path.is_symlink() {
            return Ok(path);
        }
        return Err(format!(
            "declarations not found or not a regular file: {}",
            path.display()
        ));
    }
    for cand in declaration_candidates() {
        if cand.is_file() && !cand.is_symlink() {
            if !non_interactive && is_tty() {
                if prompt_yes_no(&format!("Use declarations at {}?", cand.display()), true) {
                    return Ok(cand);
                }
            } else {
                return Ok(cand);
            }
        }
    }
    if non_interactive || !is_tty() {
        return Err(
            "declarations required; pass --declarations PATH or set SUDO_SECRETSPEC_DECLARATIONS"
                .into(),
        );
    }
    let typed = prompt_line("Path to declarations TOML", None)
        .ok_or_else(|| "declarations path required".to_string())?;
    let path = PathBuf::from(typed);
    if path.is_file() && !path.is_symlink() {
        Ok(path)
    } else {
        Err(format!("declarations not found: {}", path.display()))
    }
}

fn detect_existing_vault() -> Option<(PathBuf, String, String)> {
    // Prefer the already-deployed local vault if present.
    let candidates = [
        PathBuf::from("/var/db/stayturgid-secrets"),
        PathBuf::from("/var/db/sudo-secretspec"),
    ];
    for vault in candidates {
        if vault.is_dir() && !vault.is_symlink() {
            // Ownership can only be fully trusted after elevation; use names we know.
            if vault.ends_with("stayturgid-secrets") {
                return Some((vault, "_secretspec".into(), "staff".into()));
            }
            return Some((vault, "_sudo_secretspec".into(), "_sudo_secretspec".into()));
        }
    }
    None
}

fn run_install(
    declarations: Option<PathBuf>,
    dry_run: bool,
    adopt_existing: bool,
    vault: Option<PathBuf>,
    service_user: Option<String>,
    service_group: Option<String>,
    operator: Option<String>,
    non_interactive: bool,
) {
    let declarations = match resolve_declarations(declarations, non_interactive) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("install denied: {e}");
            std::process::exit(2);
        }
    };

    let mut req = sudo_secretspec_cli::install::InstallRequest::from_cli(
        declarations,
        dry_run,
        adopt_existing,
    );

    if let Some(existing) = detect_existing_vault() {
        if vault.is_none() {
            if adopt_existing
                || (!non_interactive
                    && is_tty()
                    && prompt_yes_no(
                        &format!(
                            "Found existing vault at {}. Adopt it?",
                            existing.0.display()
                        ),
                        true,
                    ))
            {
                req.adopt_existing = true;
                req.vault = existing.0;
                req.service_user = existing.1;
                req.service_group = existing.2;
            }
        }
    }

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
    } else if !non_interactive && is_tty() {
        if let Some(o) = prompt_line("Operator username for sudoers", Some(&req.operator)) {
            req.operator = o;
        }
    }

    // Install itself needs interactive sudo/Touch ID, not NOPASSWD -n.
    if unsafe { libc::geteuid() } != 0 {
        let status = Command::new(SUDO)
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
            .arg("--non-interactive")
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

fn lifecycle(op: &str, name: &str, reason: &str) {
    let mut cmd = Command::new(SUDO);
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
    // `command` is already exactly the target argv: clap consumed the `--`
    // delimiter via `last = true`. Any further `--` belongs to the target
    // (`cargo test -- --nocapture`) and must be passed through untouched.
    let target = command;
    if target.is_empty() {
        eprintln!("run requires a command after --");
        std::process::exit(2);
    }

    let output = Command::new(SUDO)
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
    // `vars_os`, not `vars`: the latter panics on a non-UTF-8 environment.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("SECRETSPEC_") {
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
    // Privileged checks (sudoers/vault) need root. Prefer NOPASSWD libexec.
    if unsafe { libc::geteuid() } != 0 {
        let broker = privileged_broker();
        let mut cmd = Command::new(SUDO);
        cmd.arg("-n").arg(&broker).arg("doctor");
        if json {
            cmd.arg("--json");
        }
        if let Some(cfg) = &config {
            cmd.arg("--config").arg(cfg);
        }
        match cmd.status() {
            Ok(s) if s.success() => return,
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("cannot elevate doctor via {}: {e}", broker.display());
                // Fall through to unprivileged best-effort.
            }
        }
    }

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
        // Advisories never fail the check, but the operator still needs to see
        // them; they are the only findings that can appear alongside OK.
        for finding in report.findings.iter().filter(|f| f.advisory) {
            print_finding("advisory", finding);
        }
    } else {
        eprintln!("doctor: FAILED");
        for finding in &report.findings {
            print_finding(
                if finding.advisory {
                    "advisory"
                } else {
                    "error"
                },
                finding,
            );
        }
        std::process::exit(1);
    }
}

fn print_finding(label: &str, finding: &sudo_secretspec_cli::Finding) {
    match &finding.path {
        Some(path) => eprintln!(
            "- [{label}] {} ({}): {}",
            finding.code, path, finding.detail
        ),
        None => eprintln!("- [{label}] {}: {}", finding.code, finding.detail),
    }
}
