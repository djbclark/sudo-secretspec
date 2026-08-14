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
use std::os::unix::fs::{MetadataExt, PermissionsExt};

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

fn uid_for_user(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: `getpwnam` is called with a valid NUL-terminated string and the
    // returned pointer is only dereferenced while non-null.
    unsafe {
        let pw = libc::getpwnam(c.as_ptr());
        if pw.is_null() {
            None
        } else {
            Some((*pw).pw_uid)
        }
    }
}

/// Validate the privilege boundary before any operation, and return the uid the
/// protected state must be owned by.
///
/// Existence and symlink checks alone are not a boundary: they say nothing
/// about who can rewrite the manifest that decides which secrets exist, or the
/// dotenv that holds their values. Ownership, mode, and the resolved path chain
/// are all enforced here rather than being left to `doctor`, which is
/// out-of-band, non-repairing, and may never have run.
fn require_boundary(cfg: &Config) -> Result<u32, i32> {
    let vault = &cfg.vault;
    let meta = std::fs::symlink_metadata(vault).map_err(|e| {
        eprintln!("broker: vault unreadable: {e}");
        2
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        eprintln!("broker: vault missing, symlinked, or not a directory");
        return Err(2);
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        eprintln!("broker: vault must be mode 0700");
        return Err(2);
    }

    let service_uid = uid_for_user(&cfg.service_user).ok_or_else(|| {
        eprintln!("broker: unknown service user {}", cfg.service_user);
        2
    })?;
    if meta.uid() != service_uid {
        eprintln!("broker: vault owner is not the configured service user");
        return Err(2);
    }

    // The public `/var/db/...` spelling must resolve to the pinned private
    // chain; anything else means the vault was moved or redirected.
    match vault.canonicalize() {
        Ok(resolved) if resolved == cfg.vault_realpath => {}
        Ok(resolved) => {
            eprintln!(
                "broker: vault resolves to {}, expected {}",
                resolved.display(),
                cfg.vault_realpath.display()
            );
            return Err(2);
        }
        Err(e) => {
            eprintln!("broker: vault path cannot be resolved: {e}");
            return Err(2);
        }
    }

    for name in ["secretspec.toml", ".env"] {
        let path = vault.join(name);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| {
            eprintln!("broker: {name} unreadable: {e}");
            2
        })?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            eprintln!("broker: {name} missing, symlinked, or not a regular file");
            return Err(2);
        }
        if meta.uid() != service_uid {
            eprintln!("broker: {name} owner is not the configured service user");
            return Err(2);
        }
        // Exactly 0600, not merely "nothing for group or world". The looser
        // rule also admitted 0700 and 0400, while `drift` already required
        // exactly 0600 — leaving the *enforcing* side more permissive than the
        // *reporting* one, which is backwards. A fresh install writes 0600
        // (`install::run`) and an adopted vault is checked here, so there is no
        // mode this rejects that the installer would have produced.
        if meta.permissions().mode() & 0o777 != 0o600 {
            eprintln!("broker: {name} must be mode 0600");
            return Err(2);
        }
    }
    Ok(service_uid)
}

/// `chown(2)` on a path, without following a final symlink.
///
/// `lchown` rather than `chown`: the caller has just created this path and a
/// symlink appearing at it would otherwise redirect a root-owned ownership
/// change onto whatever it points at.
fn chown(path: &std::path::Path, uid: u32, gid: u32) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("path contains an interior NUL"))?;
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
    if unsafe { libc::lchown(c_path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error());
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
    fn begin(cfg: &Config, transaction: uuid::Uuid, uid: u32, gid: u32) -> Result<Self, i32> {
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
            // `fs::copy` carries the mode across but *not* the owner, so a copy
            // made by this root process lands root-owned inside a vault owned by
            // the service user. `drift` checks the vault entry by entry, and a
            // root-owned entry there turns the deliberately-advisory
            // PENDING_ROLLBACK into a hard METADATA_MISMATCH — which fails
            // `doctor`, which every agent is told to treat as a stop. A crashed
            // mutation would then wedge the host.
            chown(&dst, uid, gid).map_err(|e| {
                eprintln!("broker: cannot set rollback copy ownership: {e}");
                let _ = std::fs::remove_file(&dst);
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
// Ambient environment
// ---------------------------------------------------------------------------

/// True for any variable that could steer the engine's control plane.
///
/// A predicate rather than a list, because a list is only correct on the day it
/// is written. The engine reads roughly twenty `SECRETSPEC_*` knobs today, four
/// of which (`SECRETSPEC_OPCLI_PATH` and its `BWS`/`PASSBOLT`/`PROTONPASS`
/// siblings) name an executable it will spawn — and this process is root. The
/// `XDG_*` family is here for the same reason: it decides which directory the
/// engine's user-global config is read from.
pub(crate) fn is_ambient_control_var(key: &str) -> bool {
    key.starts_with("SECRETSPEC_") || key.starts_with("XDG_")
}

/// Drop every ambient variable that could redirect the engine, then pin the
/// environment its user-global config resolves from.
///
/// Called at the top of [`run`], before the operation is even dispatched:
/// nothing between here and execution has any business consulting ambient
/// state, so there is no window in which a later reader could see the caller's
/// values.
fn purge_ambient_env() {
    // `vars_os`, not `vars`: the latter panics on a non-UTF-8 environment, and
    // the caller controls this environment entirely.
    for (key, _) in std::env::vars_os() {
        if is_ambient_control_var(&key.to_string_lossy()) {
            // SAFETY: single-threaded broker process; no concurrent env readers.
            unsafe { std::env::remove_var(&key) }
        }
    }

    // Clearing `XDG_CONFIG_HOME` is not enough on its own: the engine resolves
    // its user-global config through etcetera's XDG strategy, which falls back
    // to `$HOME/.config`, and `sudo` on this platform hands the *caller's*
    // `HOME` to the broker. Left alone, a root process reads a config file an
    // unprivileged caller can write — one whose `[audit] path` aims a root
    // writer at any absolute path, and whose `[defaults] profile` picks which
    // profile of the protected manifest resolves. Root's own home is the only
    // one inside the boundary.
    //
    // `set` rather than `remove`: with `HOME` unset, etcetera falls back to
    // `getpwuid(0)`, which is `/var/root` here anyway — but if that directory
    // lookup ever failed, `GlobalConfig::load()` would error and fail the
    // broker closed on a healthy system. An explicit value makes the resolved
    // path a constant rather than a directory-service round trip.
    //
    // SAFETY: single-threaded broker process; no concurrent env readers.
    unsafe {
        std::env::set_var("HOME", "/var/root");
    }
}

/// Map what happened during a mediated operation onto the terminal audit event.
///
/// Split out of [`run`] because it is the part of the funnel that can actually
/// be tested. The failures it classifies need a root process and a real vault
/// to provoke — a rollback-path collision needs a colliding fresh UUID, and
/// root ignores mode bits, so the copy cannot be made to fail by permissions —
/// but the mapping from "what happened" to "what the ledger records" is pure.
///
/// `attempted` is the name list from the attempt event: on the error path
/// `execute` never ran, so it never reported names of its own.
fn classify_terminal(
    outcome: Result<(u8, bool, Vec<String>), i32>,
    attempted: &[String],
) -> (Outcome, u8, Vec<String>) {
    match outcome {
        // The mutation could not be rolled back. The store's state is genuinely
        // unknown, which is neither success nor a clean failure.
        Ok((_, true, names)) => (Outcome::Unknown, 125, names),
        Ok((0, false, names)) => (Outcome::Success, 0, names),
        Ok((rc, false, names)) => (Outcome::Failure, rc, names),
        // Failed before `execute` ran: nothing was attempted against the store
        // and nothing was mutated. This arm is the whole point of the funnel —
        // it used to return bare, leaving an attempt with no terminal event.
        Err(code) => (
            Outcome::Failure,
            u8::try_from(code).unwrap_or(2),
            attempted.to_vec(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn run(broker: &Broker) -> Result<(), i32> {
    purge_ambient_env();
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
            let service_uid = require_boundary(&cfg)?;
            // The group the vault itself carries, not the one named in the
            // config: `audit::open_connection` already reassigns the ledger to
            // the vault's own gid, and rollback backups must land beside it
            // under the same identity.
            let service_gid = std::fs::metadata(&cfg.vault)
                .map_err(|e| {
                    eprintln!("broker: vault unreadable: {e}");
                    2
                })?
                .gid();

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
                    expected_uid: Some(service_uid),
                },
            )
            .map_err(|e| {
                eprintln!("broker: audit attempt failed: {e}");
                2
            })?;

            // Everything between the attempt and the terminal event runs inside
            // this closure so that *no* path can leave an attempt unterminated.
            // `Mutation::begin` used to `?` straight out of `run`, which left
            // exactly that: an attempt with no outcome, indistinguishable in the
            // ledger from a broker killed mid-operation. Funnelling makes the
            // guarantee structural instead of something each new `?` has to
            // remember.
            let outcome = (|| -> Result<(u8, bool, Vec<String>), i32> {
                let mutation = if matches!(
                    broker.operation.as_str(),
                    "source-set" | "source-add" | "source-delete"
                ) {
                    Some(Mutation::begin(
                        &cfg,
                        transaction,
                        service_uid,
                        service_gid,
                    )?)
                } else {
                    None
                };

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

                Ok((rc, unknown, names))
            })();

            // Terminal audit must also succeed.
            let (phase, terminal_rc, names) = classify_terminal(outcome, &names);
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
                    expected_uid: Some(service_uid),
                },
            )
            .map_err(|e| {
                // Fail closed, but do not report this as a plain policy error:
                // the operation itself has already committed or rolled back, and
                // only the ledger is incomplete. 126 distinguishes "outcome
                // unrecorded" from an operation that simply failed.
                eprintln!("broker: audit terminal failed: {e}");
                eprintln!(
                    "broker: operation outcome was rc={terminal_rc} but could not be recorded"
                );
                126
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
    // Assert who the ledger must belong to, but deliberately *without*
    // `require_boundary`. This is the one command whose job is to prove the
    // ledger is intact, and running the full boundary check first would mean a
    // drifted install (wrong binary mode, missing manifest) could no longer
    // verify its own ledger — precisely when the answer matters most. Passing
    // `None` here, though, skipped the owner comparison in both
    // `check_protected_dir` and `check_ledger_metadata`, so "verified" said
    // nothing about ownership at all.
    let expected_uid = uid_for_user(&cfg.service_user).ok_or_else(|| {
        eprintln!("broker: unknown service user {}", cfg.service_user);
        2
    })?;
    match audit::verify(&cfg.vault, Some(expected_uid)) {
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
    // The ambient environment was purged in `run` before dispatch; see
    // `purge_ambient_env`.
    let manifest = cfg.vault.join("secretspec.toml");
    let dotenv = cfg.vault.join(".env");
    // Pin dotenv provider to the protected vault file — never cwd-relative `.env`.
    // Absolute paths need the dotenv:/// form.
    let provider = format!("dotenv://{}", dotenv.display());
    let secrets = match secretspec::Secrets::load_from(&manifest) {
        Ok(mut s) => {
            s.set_provider(provider);
            // Pin the profile from the protected config. Without this the
            // engine's `resolve_profile_name` falls through to its user-global
            // config — which, inside a root process, is the caller's file.
            s.set_profile(cfg.profile.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `SECRETSPEC_*` name the engine reads, as of the pinned upstream.
    ///
    /// The predicate is what ships, so this list is a *witness* rather than the
    /// implementation: it proves the prefix rule covers the names that exist
    /// today, and a name added upstream tomorrow is covered without an edit
    /// here. The four `*_CLI_PATH` entries are the reason this matters — each
    /// names an executable the engine spawns, and the broker is root.
    const ENGINE_VARS: &[&str] = &[
        "SECRETSPEC_AGENT",
        "SECRETSPEC_BWS_CLI_PATH",
        "SECRETSPEC_FILE",
        "SECRETSPEC_KDBX_PASSWORD",
        "SECRETSPEC_OPCLI_PATH",
        "SECRETSPEC_PASSBOLT_CLI_PATH",
        "SECRETSPEC_PASSBOLT_PASSPHRASE",
        "SECRETSPEC_PASSBOLT_PRIVATE_KEY",
        "SECRETSPEC_PASSBOLT_PRIVATE_KEY_FILE",
        "SECRETSPEC_PASSBOLT_SERVER",
        "SECRETSPEC_PROFILE",
        "SECRETSPEC_PROTONPASS_CLI_PATH",
        "SECRETSPEC_PROVIDER",
        "SECRETSPEC_PROVIDER_CONCURRENCY",
        "SECRETSPEC_REASON",
        "SECRETSPEC_SCOPE",
    ];

    /// The XDG names etcetera consults. `XDG_CONFIG_HOME` is the one that
    /// decides where the engine's user-global config is read from; the rest are
    /// covered by the same prefix so none of them can be the next surprise.
    const XDG_VARS: &[&str] = &[
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
        "XDG_STATE_HOME",
    ];

    // The predicate is tested directly rather than by exercising
    // `purge_ambient_env`: `std::env::set_var` is process-global and the test
    // harness is multi-threaded, so mutating the environment to observe the
    // loop would race every other test in this binary.
    #[test]
    fn ambient_control_predicate_covers_every_engine_and_xdg_variable() {
        for name in ENGINE_VARS.iter().chain(XDG_VARS) {
            assert!(
                is_ambient_control_var(name),
                "{name} must be purged before the engine runs"
            );
        }
    }

    /// Every arm must produce a terminal event. An attempt with no terminal is
    /// indistinguishable in the ledger from a broker killed mid-operation, so
    /// "some code path returns early" is the failure this guards against.
    #[test]
    fn every_outcome_maps_to_exactly_one_terminal_phase() {
        let attempted = vec!["DATABASE_URL".to_string()];
        let done = vec!["API_KEY".to_string()];

        let cases = [
            (Ok((0, false, done.clone())), Outcome::Success, 0, &done),
            (Ok((1, false, done.clone())), Outcome::Failure, 1, &done),
            // Rollback failed: the store's state is unknown, not merely failed.
            (Ok((1, true, done.clone())), Outcome::Unknown, 125, &done),
            (Ok((0, true, done.clone())), Outcome::Unknown, 125, &done),
            // The path that previously returned with no terminal event at all.
            (Err(2), Outcome::Failure, 2, &attempted),
        ];

        for (outcome, want_phase, want_rc, want_names) in cases {
            let (phase, rc, names) = classify_terminal(outcome, &attempted);
            assert_eq!(phase, want_phase);
            assert_eq!(rc, want_rc);
            assert_eq!(&names, want_names);
        }
    }

    #[test]
    fn an_out_of_range_error_code_still_terminates_the_attempt() {
        // A terminal event is required to carry a result code, so the mapping
        // must not be able to produce "no code" for a code it cannot represent.
        let (phase, rc, _) = classify_terminal(Err(i32::MAX), &[]);
        assert_eq!(phase, Outcome::Failure);
        assert_eq!(rc, 2);
    }

    #[test]
    fn ambient_control_predicate_leaves_unrelated_variables_alone() {
        // `SUDO_USER` in particular: the broker reads it to attribute the audit
        // event, so purging it would blind the ledger.
        for name in ["SUDO_USER", "HOME", "PATH", "TERM", "SECRETSPE", "XDG"] {
            assert!(
                !is_ambient_control_var(name),
                "{name} must survive the purge"
            );
        }
    }
}
