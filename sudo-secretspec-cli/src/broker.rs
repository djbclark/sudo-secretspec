//! Root-owned fail-closed SecretSpec broker.
//!
//! The hidden `__broker` subcommand receives operation + options as trailing
//! arguments and validates the privilege boundary before running any operation.
//! All provider operations are value-free audited.
//!
//! Mutation safety: before any set/add/delete, the manifest and dotenv are
//! backed up to `.rollback.<transaction>` copies. On success the backups are
//! removed; on failure they are restored atomically. A crash during mutation
//! leaves the backups visible to `doctor` but the original state recoverable.

use std::ffi::OsString;

use clap::Parser;
use sha2::{Digest, Sha256};

use crate::audit::{self, AppendEventRequest, ClientFamily, Outcome};
use crate::config::Config;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const CONFIG_PATH: &str = "/usr/local/etc/sudo-secretspec.toml";

// ---------------------------------------------------------------------------
// Broker CLI (sub-parser for __broker)
// ---------------------------------------------------------------------------

#[derive(Debug, Parser)]
#[command(
    about = "Internal broker (sudo-only, hidden)",
    override_usage = "sudo sudo-secretspec __broker <OPERATION> [OPTIONS]"
)]
pub(crate) struct Broker {
    /// Broker operation
    #[arg()]
    pub(crate) operation: String,

    #[arg(long, default_value = "unknown")]
    pub(crate) client: String,

    #[arg(long, default_value = "")]
    pub(crate) reason: String,

    #[arg(long)]
    pub(crate) name: Option<String>,

    #[arg(long)]
    pub(crate) command_basename: Option<String>,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Parse broker args from raw trailing arguments and route to the operation.
pub fn dispatch(raw: &[OsString]) -> Result<(), i32> {
    let broker = match Broker::try_parse_from(
        std::iter::once(OsString::from("__broker")).chain(raw.iter().cloned()),
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("broker: {e}");
            return Err(2);
        }
    };
    run(&broker)
}

// ---------------------------------------------------------------------------
// Environment validation
// ---------------------------------------------------------------------------

fn require_root() -> Result<(), i32> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("broker: must run as root");
        return Err(2);
    }
    Ok(())
}

fn load_config() -> Result<Config, i32> {
    let content = std::fs::read_to_string(CONFIG_PATH).map_err(|e| {
        eprintln!("broker: cannot read config: {e}");
        2
    })?;
    Config::parse(&content).map_err(|e| {
        eprintln!("broker: invalid config: {e}");
        2
    })
}

fn require_boundary(cfg: &Config) -> Result<(), i32> {
    let vault = &cfg.vault;
    if !vault.exists() || vault.is_symlink() {
        eprintln!("broker: vault missing or symlinked");
        return Err(2);
    }
    let manifest = vault.join("secretspec.toml");
    let dotenv = vault.join(".env");
    if !manifest.is_file() || manifest.is_symlink() {
        eprintln!("broker: manifest missing or symlinked");
        return Err(2);
    }
    if !dotenv.is_file() || dotenv.is_symlink() {
        eprintln!("broker: dotenv missing or symlinked");
        return Err(2);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mutation helpers — backup/restore/commit for manifest and dotenv
// ---------------------------------------------------------------------------

struct Mutation {
    vault: std::path::PathBuf,
    manifest: std::path::PathBuf,
    dotenv: std::path::PathBuf,
    transaction: uuid::Uuid,
}

impl Mutation {
    fn begin(cfg: &Config, transaction: uuid::Uuid) -> Result<Self, i32> {
        let vault = cfg.vault.clone();
        let manifest = vault.join("secretspec.toml");
        let dotenv = vault.join(".env");

        let m = Self {
            vault,
            manifest,
            dotenv,
            transaction,
        };

        // Create rollback copies
        for (src, suffix) in [(&m.manifest, "toml"), (&m.dotenv, "env")] {
            let dst = m.rollback_path(suffix);
            if dst.exists() {
                eprintln!("broker: rollback path collision: {}", dst.display());
                return Err(2);
            }
            std::fs::copy(src, &dst).map_err(|e| {
                eprintln!("broker: cannot create rollback copy: {e}");
                2
            })?;
        }
        Ok(m)
    }

    fn rollback_path(&self, suffix: &str) -> std::path::PathBuf {
        self.vault.join(format!(
            ".secretspec.{}.rollback.{}",
            suffix, self.transaction
        ))
    }

    fn restore(&self) -> bool {
        let mut ok = true;
        for (dest, suffix) in [(&self.manifest, "toml"), (&self.dotenv, "env")] {
            let backup = self.rollback_path(suffix);
            if backup.is_file() {
                if std::fs::copy(&backup, dest).is_err() {
                    ok = false;
                    continue;
                }
                let _ = std::fs::remove_file(&backup);
            } else {
                ok = false;
            }
        }
        ok
    }

    fn commit(&self) {
        for suffix in ["toml", "env"] {
            let backup = self.rollback_path(suffix);
            let _ = std::fs::remove_file(&backup);
        }
    }
}

// ---------------------------------------------------------------------------
// Client resolution
// ---------------------------------------------------------------------------

fn resolve_client(raw: &str) -> ClientFamily {
    match raw {
        "hermes" => ClientFamily::Hermes,
        "claude" => ClientFamily::Claude,
        "codex" => ClientFamily::Codex,
        "opencode" => ClientFamily::OpenCode,
        "cursor" => ClientFamily::Cursor,
        "agy" => ClientFamily::Agy,
        "fixed-consumer" => ClientFamily::FixedConsumer,
        _ => ClientFamily::Unknown,
    }
}

fn reason_sha256(reason: &str) -> Option<String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(reason.as_bytes())))
}

fn actor_from_env() -> String {
    std::env::var("SUDO_USER").unwrap_or_else(|_| "root".into())
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn run(broker: &Broker) -> Result<(), i32> {
    match broker.operation.as_str() {
        "audit-verify" => run_audit_verify(broker),
        "source-get"
        | "source-set"
        | "source-add"
        | "source-delete"
        | "source-check"
        | "source-export"
        | "source-template-check" => {
            require_root()?;
            let cfg = load_config()?;
            require_boundary(&cfg)?;

            if broker.reason.trim().is_empty() {
                eprintln!("broker: --reason is required");
                return Err(2);
            }

            let client = resolve_client(&broker.client);
            let reason_hash = reason_sha256(&broker.reason).ok_or(2)?;
            let transaction = uuid::Uuid::new_v4();
            let names = broker
                .name
                .as_deref()
                .map(|n| vec![n.to_string()])
                .unwrap_or_default();
            let actor = actor_from_env();

            // Fail-closed audit attempt before any secret operation.
            audit::append_event(
                &cfg.vault,
                AppendEventRequest {
                    operation: broker.operation.clone(),
                    phase: Outcome::Attempt,
                    transaction,
                    actor: actor.clone(),
                    client,
                    reason_sha256: Some(reason_hash.clone()),
                    command_basename: broker.command_basename.clone(),
                    names: names.clone(),
                    result_code: None,
                    expected_uid: None,
                },
            )
            .map_err(|e| {
                eprintln!("broker: audit attempt failed: {e}");
                2
            })?;

            // Begin mutation for write operations
            let mutation = if matches!(
                broker.operation.as_str(),
                "source-set" | "source-add" | "source-delete"
            ) {
                Some(Mutation::begin(&cfg, transaction)?)
            } else {
                None
            };

            // Execute the operation
            let (rc, names) = execute(broker, &cfg);

            // Commit or restore mutation
            let mut unknown = false;
            if let Some(m) = &mutation {
                if rc == 0 {
                    m.commit();
                } else if !m.restore() {
                    unknown = true;
                }
            }

            // Terminal audit must also succeed.
            let phase = if unknown {
                Outcome::Unknown
            } else if rc == 0 {
                Outcome::Success
            } else {
                Outcome::Failure
            };
            let terminal_rc = if unknown { 125 } else { rc };
            audit::append_event(
                &cfg.vault,
                AppendEventRequest {
                    operation: broker.operation.clone(),
                    phase,
                    transaction,
                    actor,
                    client,
                    reason_sha256: Some(reason_hash),
                    command_basename: broker.command_basename.clone(),
                    names,
                    result_code: Some(terminal_rc),
                    expected_uid: None,
                },
            )
            .map_err(|e| {
                eprintln!("broker: audit terminal failed: {e}");
                2
            })?;

            if terminal_rc != 0 {
                Err(i32::from(terminal_rc))
            } else {
                Ok(())
            }
        }
        op => {
            eprintln!("broker: unknown operation: {op}");
            Err(2)
        }
    }
}

fn run_audit_verify(_broker: &Broker) -> Result<(), i32> {
    require_root()?;
    let cfg = load_config()?;
    match audit::verify(&cfg.vault, None) {
        Ok(result) => {
            println!("audit-verify: {} events, tip {}", result.count, result.hash);
            Ok(())
        }
        Err(e) => {
            eprintln!("audit-verify failed: {e}");
            Err(1)
        }
    }
}

fn execute(broker: &Broker, cfg: &Config) -> (u8, Vec<String>) {
    // Prevent ambient SecretSpec env from selecting another control plane.
    for key in [
        "SECRETSPEC_PROVIDER",
        "SECRETSPEC_FILE",
        "SECRETSPEC_PROFILE",
        "SECRETSPEC_SCOPE",
    ] {
        // SAFETY: single-threaded broker process; no concurrent env readers.
        unsafe {
            std::env::remove_var(key);
        }
    }

    let manifest = cfg.vault.join("secretspec.toml");
    let dotenv = cfg.vault.join(".env");
    // Pin dotenv provider to the protected vault file — never cwd-relative `.env`.
    // Absolute paths need the dotenv:/// form.
    let provider = format!("dotenv://{}", dotenv.display());
    let secrets = match secretspec::Secrets::load_from(&manifest) {
        Ok(mut s) => {
            s.set_provider(provider);
            s = s.with_reason(broker.reason.clone());
            s
        }
        Err(e) => {
            eprintln!("broker: cannot load secrets: {e}");
            return (2, vec![]);
        }
    };

    let name = broker.name.as_deref().unwrap_or("");

    match broker.operation.as_str() {
        "source-get" => match secrets.resolve_named(name) {
            Ok(secretspec::NamedResolution::Resolved(secret)) => {
                if let Some(value) = secret.value {
                    println!("{value}");
                    (0, vec![name.into()])
                } else {
                    eprintln!("broker: {name} has no value");
                    (1, vec![name.into()])
                }
            }
            Ok(_) => {
                eprintln!("broker: {name} is not resolved");
                (1, vec![name.into()])
            }
            Err(e) => {
                eprintln!("broker: {e}");
                (1, vec![name.into()])
            }
        },
        "source-set" | "source-add" => match secrets.set(name, None) {
            Ok(()) => (0, vec![name.into()]),
            Err(e) => {
                eprintln!("broker: {e}");
                (1, vec![name.into()])
            }
        },
        "source-delete" => match secrets.delete(name) {
            Ok(_) => (0, vec![name.into()]),
            Err(e) => {
                eprintln!("broker: {e}");
                (1, vec![name.into()])
            }
        },
        "source-check" => match secrets.check(false) {
            Ok(_) => (0, vec![]),
            Err(e) => {
                eprintln!("broker: {e}");
                (1, vec![])
            }
        },
        "source-export" => {
            use std::io;
            match secrets.export(secretspec::ExportFormat::Json, &mut io::stdout().lock()) {
                Ok(()) => (0, vec![]),
                Err(e) => {
                    eprintln!("broker: {e}");
                    (1, vec![])
                }
            }
        }
        "source-template-check" => {
            let declarations = &cfg.declarations;
            if !declarations.is_file() || declarations.is_symlink() {
                eprintln!("broker: declaration template missing or symlinked");
                return (2, vec![]);
            }
            let manifest_bytes = std::fs::read(&manifest).unwrap_or_default();
            let template_bytes = std::fs::read(declarations).unwrap_or_default();
            if manifest_bytes == template_bytes {
                println!("runtime manifest matches tracked declaration example");
                (0, vec![])
            } else {
                eprintln!("broker: manifest differs from declaration template");
                (1, vec![])
            }
        }
        _ => (2, vec![]),
    }
}
