use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
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

/// Installed protected configuration, written by `install`.
const CONFIG_PATH: &str = "/usr/local/etc/sudo-secretspec.toml";

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
    /// Declare a new secret in the runtime manifest.
    ///
    /// Creates the declaration only; it assigns no value. Follow with `set`.
    ///
    /// This edits the runtime manifest immediately, so `template-check` will
    /// report drift until you either mirror the declaration into the tracked
    /// declarations file and release it, or undo it with `undeclare`.
    Add {
        name: String,
        /// Human description recorded in the declaration.
        #[arg(long)]
        description: String,
        /// Declare the secret optional, writing `required = false`.
        #[arg(long, conflicts_with = "required")]
        optional: bool,
        /// Declare the secret required, writing `required = true`. Only needed
        /// when the profile's `[defaults]` set `required = false`.
        #[arg(long)]
        required: bool,
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
    /// Remove a declaration this client added at runtime.
    ///
    /// The inverse of `add`. Refuses a name that is in the tracked declaration
    /// template — removing one of those is a review-and-release decision — and
    /// refuses a name that still holds a value, so `delete` comes first.
    Undeclare {
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
    /// Compare the runtime manifest against the tracked declaration template.
    ///
    /// Reports drift between `<vault>/secretspec.toml` (what the broker
    /// actually resolves from) and the `declarations` file recorded in the
    /// root-owned config (what review sees in git). Reads no secret values.
    TemplateCheck {
        #[arg(long)]
        reason: String,
    },
    /// Emit a JSON Schema of the runtime manifest's typed shape.
    ///
    /// Declared names and whether each is required — never values, and not
    /// descriptions either: `codegen`'s emitter writes only the `type` key
    /// per property. The profile is the one recorded in the root-owned
    /// config, not a caller flag: an arbitrary profile would enumerate
    /// shapes the boundary is not configured for.
    Schema {
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
        /// Manifest profile the broker resolves secrets from.
        ///
        /// Recorded in the root-owned config rather than read from the
        /// environment at operation time, so the profile cannot be chosen by
        /// whoever calls the broker. Defaults to `default`.
        #[arg(long)]
        profile: Option<String>,
        /// Never prompt; fail if required values cannot be defaulted.
        #[arg(long)]
        non_interactive: bool,
    },
    /// Remove the privileged boundary this installer owns.
    ///
    /// Leaves the vault and the service identity alone unless asked; both
    /// outlive any single install.
    ///
    /// Typical short form:
    ///   sudo-secretspec uninstall --dry-run
    ///   sudo-secretspec uninstall
    Uninstall {
        #[arg(long)]
        dry_run: bool,
        /// Also delete the vault directory and every secret in it.
        #[arg(long)]
        purge_vault: bool,
        /// Also delete the service user and group.
        #[arg(long)]
        remove_service_user: bool,
        #[arg(long)]
        config: Option<PathBuf>,
        /// Never prompt; the opt-in flags are taken as the confirmation.
        #[arg(long)]
        non_interactive: bool,
    },
    Doctor {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        config: Option<PathBuf>,
        /// Executable search path of the unprivileged caller.
        ///
        /// Set by the public client when it elevates: `sudo` replaces `PATH`
        /// with the policy's `secure_path`, so the privileged process cannot
        /// otherwise tell which `sudo-secretspec` the operator would run. It
        /// only widens the shadow scan and is not an operator-facing knob.
        #[arg(long, hide = true)]
        caller_path: Option<OsString>,
        /// Path of a package-manager build staged but not installed.
        ///
        /// Set by the public client alongside `--available-version`. Like
        /// `caller_path` this is a fact the privileged process cannot safely
        /// collect for itself, not an operator-facing knob.
        #[arg(long, hide = true)]
        available_path: Option<PathBuf>,
        /// Version that staged build reports.
        #[arg(long, hide = true, requires = "available_path")]
        available_version: Option<String>,
    },
    /// Verify the audit ledger's hash chain and report its tip.
    ///
    /// Reads no secret and takes no reason: its whole job is to prove the
    /// ledger is intact. Record the reported tip hash outside the vault and
    /// compare it on later runs -- that external pin is what gives
    /// tamper-evidence against a principal who can write the ledger.
    AuditVerify,
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
///
/// The two ways of not knowing are **not** the same, and collapsing them is
/// what made this fail open. Keep the asymmetry:
///
/// - `canonicalize(BROKER_PATH)` fails → no broker is installed → this process
///   cannot be it → `false`. This is what lets the Homebrew `libexec`
///   bootstrap run `install` at all, on a machine with no boundary yet.
/// - `current_exe()` fails → we cannot identify ourselves → `true`, refusing
///   boundary lifecycle. The allowlist in `main` is presented as the layer that
///   catches a sudoers mistake, so it has to fail closed when it cannot tell.
fn invoked_as_privileged_broker() -> bool {
    match (std::env::current_exe(), std::fs::canonicalize(BROKER_PATH)) {
        (Ok(me), Ok(broker)) => std::fs::canonicalize(me)
            .map(|me| me == broker)
            .unwrap_or(true),
        (Err(_), _) => true,
        (_, Err(_)) => false,
    }
}

fn main() {
    let cli = Cli::parse();

    // The libexec path is NOPASSWD for the operator. Boundary lifecycle must
    // stay behind an interactive authentication, so refuse anything other than
    // the mediated operations when we were invoked through that path.
    //
    // This is an allowlist, deliberately: `install`, `rollback` and `uninstall`
    // are refused here by not appearing, and any lifecycle command added later
    // inherits the refusal instead of having to remember to ask for it.
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
        Cmd::Add {
            name,
            description,
            optional,
            required,
            reason,
        } => lifecycle_add(&name, &description, optional, required, &reason),
        Cmd::Set { name, reason } => lifecycle("set", &name, &reason),
        Cmd::Delete { name, reason } => lifecycle("delete", &name, &reason),
        Cmd::Undeclare { name, reason } => lifecycle("undeclare", &name, &reason),
        Cmd::Get { name, reason } => lifecycle("get", &name, &reason),
        Cmd::Check { reason } => lifecycle("check", "", &reason),
        Cmd::Export { reason } => lifecycle("export", "", &reason),
        Cmd::TemplateCheck { reason } => lifecycle("template-check", "", &reason),
        Cmd::Schema { reason } => lifecycle("schema", "", &reason),
        Cmd::Run { reason, command } => run_target(&reason, &command),
        Cmd::Install {
            declarations,
            dry_run,
            adopt_existing,
            vault,
            service_user,
            service_group,
            operator,
            profile,
            non_interactive,
        } => run_install(
            declarations,
            dry_run,
            adopt_existing,
            vault,
            service_user,
            service_group,
            operator,
            profile,
            non_interactive,
        ),
        Cmd::Uninstall {
            dry_run,
            purge_vault,
            remove_service_user,
            config,
            non_interactive,
        } => run_uninstall(
            dry_run,
            purge_vault,
            remove_service_user,
            config,
            non_interactive,
        ),
        Cmd::Doctor {
            json,
            config,
            caller_path,
            available_path,
            available_version,
        } => doctor(
            json,
            config,
            caller_path,
            available_path
                .zip(available_version)
                .map(|(path, version)| sudo_secretspec_cli::AvailableBuild { path, version }),
        ),
        Cmd::AuditVerify => audit_verify(),
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
    sudo_secretspec_cli::install::detect_existing_vault(Path::new(CONFIG_PATH), |path| {
        path.is_dir() && !path.is_symlink()
    })
}

fn run_install(
    declarations: Option<PathBuf>,
    dry_run: bool,
    adopt_existing: bool,
    vault: Option<PathBuf>,
    service_user: Option<String>,
    service_group: Option<String>,
    operator: Option<String>,
    profile: Option<String>,
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
    if let Some(p) = profile {
        req.profile = p;
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
            .arg("--profile")
            .arg(&req.profile)
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

fn run_uninstall(
    dry_run: bool,
    purge_vault: bool,
    remove_service_user: bool,
    config: Option<PathBuf>,
    non_interactive: bool,
) {
    // Confirm here, while still unprivileged and still attached to the
    // operator's terminal. The elevated re-exec below passes
    // `--non-interactive`, so this is the only place a prompt can happen —
    // and both of these destroy state that outlives the install.
    if !non_interactive && !dry_run && is_tty() {
        if purge_vault
            && !prompt_yes_no(
                "Delete the vault and every secret in it? This cannot be undone.",
                false,
            )
        {
            eprintln!("uninstall aborted");
            std::process::exit(2);
        }
        if remove_service_user
            && !prompt_yes_no(
                "Delete the service user and group? Other tools may still depend on them.",
                false,
            )
        {
            eprintln!("uninstall aborted");
            std::process::exit(2);
        }
    }

    // Uninstall is boundary lifecycle: interactive sudo/Touch ID, never the
    // NOPASSWD broker path.
    if unsafe { libc::geteuid() } != 0 {
        let mut cmd = Command::new(SUDO);
        cmd.arg(self_exe()).arg("uninstall");
        if dry_run {
            cmd.arg("--dry-run");
        }
        if purge_vault {
            cmd.arg("--purge-vault");
        }
        if remove_service_user {
            cmd.arg("--remove-service-user");
        }
        if let Some(cfg) = &config {
            cmd.arg("--config").arg(cfg);
        }
        cmd.arg("--non-interactive");
        match cmd.status() {
            Ok(s) if s.success() => return,
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("cannot elevate uninstall: {e}");
                std::process::exit(2);
            }
        }
    }

    let req = sudo_secretspec_cli::uninstall::UninstallRequest {
        dry_run,
        purge_vault,
        remove_service_user,
        config: config.unwrap_or_else(|| PathBuf::from(CONFIG_PATH)),
    };
    if let Err(e) = sudo_secretspec_cli::uninstall::run(req) {
        eprintln!("uninstall denied: {e}");
        std::process::exit(2);
    }
}

/// Hash the operator's reason before it crosses the privilege boundary.
///
/// The reason used to be passed to the broker as plaintext in argv, where
/// `ps` and `KERN_PROCARGS2` expose it to every process running as the same
/// user, and where it also lands in shell history — while the design's stated
/// invariant is that only `SHA-256(reason)` is ever stored. Hashing here keeps
/// the prose inside the unprivileged process that already had it.
///
/// The non-empty check stays on this side: it must not be possible to satisfy
/// the gate by sending `SHA-256("")`, and the broker rejects that digest too.
fn reason_digest_or_exit(reason: &str) -> String {
    sudo_secretspec_cli::broker::reason_sha256(reason).unwrap_or_else(|| {
        eprintln!("sudo-secretspec: --reason must not be empty");
        std::process::exit(2);
    })
}

/// Report a broker exit, adding a hint for the one confusing failure.
fn exit_from_broker(code: i32) -> ! {
    // The client and the broker are installed as a pair, but the Homebrew keg
    // ships a bootstrap binary that can be newer than the installed broker. An
    // older broker rejects `--reason-sha256` with clap's usage exit, which
    // otherwise reads as an unexplained argument error. Deliberately *not* a
    // retry with plaintext: falling back would hand an attacker who can
    // downgrade the broker a way to get the prose back.
    if code == 2 {
        eprintln!(
            "sudo-secretspec: if the broker reported an unexpected '--reason-sha256' argument,\n\
             the installed boundary predates this client; re-run `sudo-secretspec install`"
        );
    }
    std::process::exit(code);
}

/// `add` is `lifecycle` plus the declaration's description.
///
/// Kept separate rather than widening `lifecycle` with an `Option`: `add` is
/// the only operation that carries one, and threading a `None` through every
/// other call site is how `add` came to be a silent alias for `set`.
fn lifecycle_add(name: &str, description: &str, optional: bool, required: bool, reason: &str) {
    if description.trim().is_empty() {
        eprintln!("sudo-secretspec: --description cannot be empty");
        std::process::exit(2);
    }
    let mut command = Command::new(SUDO);
    command
        .arg("-n")
        .arg(privileged_broker())
        .arg("__broker")
        .arg("source-add")
        .arg("--client")
        .arg(detect_client())
        .arg("--reason-sha256")
        .arg(reason_digest_or_exit(reason))
        .arg("--name")
        .arg(name)
        .arg("--description")
        .arg(description);
    if optional {
        command.arg("--optional");
    }
    if required {
        command.arg("--required");
    }
    let status = command.status().unwrap_or_else(|e| {
        eprintln!("cannot invoke broker: {e}");
        std::process::exit(2);
    });
    if !status.success() {
        exit_from_broker(status.code().unwrap_or(1));
    }
}

fn audit_verify() {
    let status = Command::new(SUDO)
        .arg("-n")
        .arg(privileged_broker())
        .arg("__broker")
        .arg("audit-verify")
        .status()
        .unwrap_or_else(|e| {
            eprintln!("cannot invoke broker: {e}");
            std::process::exit(2);
        });
    if !status.success() {
        exit_from_broker(status.code().unwrap_or(1));
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
        .arg("--reason-sha256")
        .arg(reason_digest_or_exit(reason));
    if !name.is_empty() {
        cmd.arg("--name").arg(name);
    }
    // `status`, not `output`: `get` streams the secret value straight to the
    // caller's stdout, and capturing it here would put a copy in this process.
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("cannot invoke broker: {e}");
        std::process::exit(2);
    });
    if !status.success() {
        exit_from_broker(status.code().unwrap_or(1));
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
        .arg("--reason-sha256")
        .arg(reason_digest_or_exit(reason))
        .arg("--command-basename")
        .arg(sudo_secretspec_cli::audit::command_basename(&target[0]))
        .output()
        .unwrap_or_else(|e| {
            eprintln!("cannot invoke broker: {e}");
            std::process::exit(2);
        });

    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        exit_from_broker(output.status.code().unwrap_or(1));
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

/// Ask the package manager's staged copy of the boundary what version it is.
///
/// Runs only while unprivileged, and that restriction is the point: the
/// Homebrew prefix is routinely operator-writable, so this is a binary the
/// *operator* may execute but the root broker must not. `doctor` collects the
/// answer here and hands it across the elevation boundary as data.
///
/// A build that cannot be run, or whose output is not a single version word, is
/// simply not reported — this feeds an advisory, and guessing would be worse
/// than staying quiet.
fn probe_available_build(unprivileged: bool) -> Option<sudo_secretspec_cli::AvailableBuild> {
    if !unprivileged {
        return None;
    }
    sudo_secretspec_cli::media_candidates()
        .into_iter()
        .find_map(|path| {
            let out = Command::new(&path).arg("--version").output().ok()?;
            if !out.status.success() {
                return None;
            }
            // `clap` prints "<name> <version>"; take the last word.
            let version = String::from_utf8(out.stdout)
                .ok()?
                .split_whitespace()
                .next_back()?
                .to_string();
            if version.is_empty() {
                return None;
            }
            Some(sudo_secretspec_cli::AvailableBuild { path, version })
        })
}

fn doctor(
    json: bool,
    config: Option<PathBuf>,
    caller_path: Option<OsString>,
    available_build: Option<sudo_secretspec_cli::AvailableBuild>,
) {
    // `sudo` replaces PATH with the policy's secure_path, so the shadow check
    // has to be told what the caller's PATH was before elevating. Trust the
    // ambient value only while still unprivileged.
    let unprivileged = unsafe { libc::geteuid() } != 0;
    let caller_path = caller_path.or_else(|| {
        if unprivileged {
            std::env::var_os("PATH")
        } else {
            None
        }
    });
    let available_build = available_build.or_else(|| probe_available_build(unprivileged));

    // Privileged checks (sudoers/vault) need root. Prefer NOPASSWD libexec.
    if unprivileged {
        let broker = privileged_broker();
        let elevated = |with_hints: bool| {
            let mut cmd = Command::new(SUDO);
            cmd.arg("-n").arg(&broker).arg("doctor");
            if json {
                cmd.arg("--json");
            }
            if let Some(cfg) = &config {
                cmd.arg("--config").arg(cfg);
            }
            if with_hints {
                if let Some(path) = &caller_path {
                    cmd.arg("--caller-path").arg(path);
                }
                if let Some(build) = &available_build {
                    cmd.arg("--available-path")
                        .arg(&build.path)
                        .arg("--available-version")
                        .arg(&build.version);
                }
            }
            cmd
        };

        // A client from a newer build can meet an older installed broker: brew
        // hands over a new bootstrap binary before `install` replaces the pair.
        // The older broker rejects these hidden flags with clap's usage exit, so
        // capture that attempt rather than letting a bare argument-parsing error
        // stand in for the health check. `doctor` itself exits 0 or 1, so a 2
        // from this path means the broker did not understand the request.
        //
        // All hints are passed and dropped together: retrying flag-by-flag would
        // multiply round trips through `sudo` to recover detail that is
        // advisory either way.
        let mut can_elevate = true;
        if caller_path.is_some() || available_build.is_some() {
            match elevated(true).output() {
                Ok(out) if out.status.code() == Some(2) => {
                    // Retry without the flag; the shadow check then runs against
                    // the fixed search list only.
                }
                Ok(out) => {
                    let _ = io::stdout().write_all(&out.stdout);
                    let _ = io::stderr().write_all(&out.stderr);
                    match out.status.code() {
                        Some(0) => return,
                        code => std::process::exit(code.unwrap_or(1)),
                    }
                }
                Err(e) => {
                    eprintln!("cannot elevate doctor via {}: {e}", broker.display());
                    can_elevate = false;
                }
            }
        }

        if can_elevate {
            match elevated(false).status() {
                Ok(s) if s.success() => return,
                Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                Err(e) => {
                    eprintln!("cannot elevate doctor via {}: {e}", broker.display());
                    // Fall through to unprivileged best-effort.
                }
            }
        }
    }

    let config = config.unwrap_or_else(|| PathBuf::from(CONFIG_PATH));
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

    let report = sudo_secretspec_cli::inspect(
        &layout,
        &sudo_secretspec_cli::InspectOptions {
            caller_path,
            available_build,
        },
    );
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
