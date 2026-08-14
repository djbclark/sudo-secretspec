//! Metadata-only, non-repairing drift validation for sudo-secretspec.
//!
//! Never reads secret values. Never mutates state.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::audit;
use crate::config::Config;

#[derive(Debug, Error)]
pub enum DriftError {
    #[error("config: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub config: PathBuf,
    pub vault: PathBuf,
    pub vault_realpath: PathBuf,
    pub service_user: String,
    pub service_group: String,
    pub declarations: PathBuf,
    pub engine: PathBuf,
    pub audit: PathBuf,
    pub broker: PathBuf,
    pub client: PathBuf,
    pub checker: PathBuf,
    pub retired: PathBuf,
    pub guidance: PathBuf,
    pub sudoers: PathBuf,
    pub source_manifest: PathBuf,
}

impl Layout {
    fn from_config(cfg: Config, config_path: PathBuf) -> Self {
        // Prefer deriving the install prefix from the configured engine path.
        // Single-binary installs place the privileged binary under libexec and
        // the public client under bin.
        let prefix = cfg
            .engine
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("/usr/local"))
            .to_path_buf();
        let libexec = prefix.join("libexec");
        let bin = prefix.join("bin");
        let share = prefix.join("share").join("sudo-secretspec");

        Self {
            vault: cfg.vault,
            vault_realpath: cfg.vault_realpath,
            service_user: cfg.service_user,
            service_group: cfg.service_group,
            declarations: cfg.declarations,
            engine: cfg.engine.clone(),
            // In the Rust single-binary model the audit helper is the same
            // privileged binary; keep a distinct field for compatibility.
            audit: cfg.audit_helper,
            broker: libexec.join("sudo-secretspec"),
            client: bin.join("sudo-secretspec"),
            checker: bin.join("sudo-secretspec"),
            retired: share.join("sudo-secretspec-retired.toml"),
            guidance: share.join("AI-GUIDANCE.md"),
            sudoers: PathBuf::from("/private/etc/sudoers.d/sudo-secretspec"),
            source_manifest: share.join("MANIFEST.sha256"),
            config: config_path,
        }
    }
}

/// Findings that require operator review but do not invalidate the boundary.
///
/// Both describe state the operator must clean up by hand — non-secret tool
/// state left under the vault, and a mutation backup a crash left behind — and
/// neither means a credential operation would be unsafe. They must not fail
/// `doctor`: agents are instructed to treat a drift failure as a hard stop, so
/// a permanent advisory would wedge every automated caller indefinitely.
const ADVISORY_CODES: &[&str] = &[
    "LEGACY_VAULT_CLUTTER",
    "PENDING_ROLLBACK",
    "CLIENT_DUPLICATE",
];

/// Directories scanned unconditionally for a second copy of the public client.
///
/// `doctor` re-execs itself as root through `sudo`, and the policy this project
/// installs replaces `PATH` with `secure_path` (`install::sudoers_text`). The
/// privileged process therefore cannot see the `PATH` whose shadowing we care
/// about, and resolving the client through its own `PATH` would answer a
/// question nobody asked. These are the conventional macOS search directories,
/// checked from a fixed list so the result never depends on a caller-supplied
/// value.
const SEARCH_DIRS: &[&str] = &[
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/opt/local/bin",
    "/opt/local/sbin",
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
];

/// Inputs `inspect` cannot obtain for itself from inside the privileged process.
#[derive(Debug, Clone, Default)]
pub struct InspectOptions {
    /// Executable search path of the unprivileged caller, when known.
    ///
    /// Supplied by the public client before it elevates, because `sudo`
    /// discards the caller's `PATH`. It can only *widen* the shadow scan:
    /// [`SEARCH_DIRS`] is always covered, so a caller passing a doctored value
    /// can add findings but never hide one.
    pub caller_path: Option<OsString>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Advisory findings are reported but do not clear `Report.ok`.
    pub advisory: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Report {
    pub ok: bool,
    pub findings: Vec<Finding>,
}

impl Report {
    fn from_findings(findings: Vec<Finding>) -> Self {
        Self {
            ok: findings.iter().all(|f| f.advisory),
            findings,
        }
    }
}

fn finding(code: &str, path: Option<&Path>, detail: impl Into<String>) -> Finding {
    Finding {
        code: code.into(),
        detail: detail.into(),
        path: path.map(|p| p.display().to_string()),
        advisory: ADVISORY_CODES.contains(&code),
    }
}

/// Load and validate the protected TOML configuration at `path`.
pub fn load_config(path: &Path) -> Result<Layout, DriftError> {
    let content = fs::read_to_string(path)?;
    let cfg = Config::parse(&content)?;
    Ok(Layout::from_config(cfg, path.to_path_buf()))
}

fn owner_name(uid: u32) -> String {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return uid.to_string();
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_string_lossy()
            .into_owned()
    }
}

fn group_name(gid: u32) -> String {
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            return gid.to_string();
        }
        std::ffi::CStr::from_ptr((*gr).gr_name)
            .to_string_lossy()
            .into_owned()
    }
}

fn metadata_owner_group_mode(path: &Path) -> std::io::Result<(String, String, u32)> {
    let meta = fs::symlink_metadata(path)?;
    Ok((
        owner_name(meta.uid()),
        group_name(meta.gid()),
        meta.permissions().mode() & 0o777,
    ))
}

fn has_extended_acl(path: &Path) -> bool {
    let output = Command::new("/bin/ls")
        .args(["-lde", &path.display().to_string()])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).lines().count() != 1
        }
        _ => true,
    }
}

fn check_protected_file(
    path: &Path,
    owner: &str,
    group: &str,
    mode: u32,
    findings: &mut Vec<Finding>,
) {
    if path.is_symlink() {
        findings.push(finding(
            "SYMLINK_FORBIDDEN",
            Some(path),
            "path must not be a symlink",
        ));
        return;
    }
    match metadata_owner_group_mode(path) {
        Ok((o, g, m)) => {
            if o != owner || g != group || m != mode {
                findings.push(finding(
                    "METADATA_MISMATCH",
                    Some(path),
                    format!("expected {owner}:{group}:{mode:o}, got {o}:{g}:{m:o}"),
                ));
            }
            if has_extended_acl(path) {
                findings.push(finding(
                    "ACL_PRESENT",
                    Some(path),
                    "extended ACL present on protected path",
                ));
            }
        }
        Err(_) => findings.push(finding(
            "PATH_MISSING",
            Some(path),
            "required protected path is missing",
        )),
    }
}

fn check_ancestor_chain(path: &Path, findings: &mut Vec<Finding>) {
    // Validate the resolved path's parents for root ownership and non-writability.
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut current = resolved.as_path();
    while let Some(parent) = current.parent() {
        if parent == Path::new("/") {
            break;
        }
        // Accept the conventional macOS alias roots only as leaf public names;
        // validation always walks the resolved private chain.
        if parent.is_symlink()
            && !matches!(parent.to_string_lossy().as_ref(), "/var" | "/etc" | "/tmp")
        {
            findings.push(finding(
                "ANCESTOR_SYMLINK",
                Some(parent),
                "protected ancestor must not be a symlink",
            ));
        }
        if let Ok((o, _g, m)) = metadata_owner_group_mode(parent) {
            if o != "root" || m & 0o022 != 0 {
                findings.push(finding(
                    "REMOVABLE_ANCESTOR",
                    Some(parent),
                    "ancestor is not root-owned or is group/world-writable",
                ));
            }
            if has_extended_acl(parent) {
                findings.push(finding(
                    "ACL_PRESENT",
                    Some(parent),
                    "extended ACL present on ancestor",
                ));
            }
        }
        current = parent;
        if parent == Path::new("/private") || parent == Path::new("/usr") {
            break;
        }
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

/// Parse `hash  absolute-path` rows. An unreadable manifest yields no rows,
/// which every caller must treat as "cannot prove anything about these paths"
/// rather than "these paths are fine".
pub(crate) fn parse_manifest(path: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (hash, rest) = line.split_once(char::is_whitespace)?;
            let rest = rest.trim();
            Some((hash.to_string(), PathBuf::from(rest)))
        })
        .collect()
}

/// Ordered directories to scan for a second copy of the client.
///
/// The fixed list comes first and is never removable; `caller_path` only
/// appends. Relative entries are dropped: `PATH` may legitimately contain them,
/// but resolving one inside a root process would depend on its working
/// directory, which the caller also controls.
fn search_dirs(caller_path: Option<&OsStr>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = SEARCH_DIRS.iter().map(PathBuf::from).collect();
    if let Some(path) = caller_path {
        dirs.extend(std::env::split_paths(path).filter(|p| p.is_absolute()));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// The first `name` the caller's `PATH` would execute, when that `PATH` is
/// known. `None` when it was not supplied or resolves to nothing.
fn path_winner(caller_path: Option<&OsStr>, name: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(caller_path?)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|c| c.symlink_metadata().is_ok())
}

/// Nearest ancestor of `dir` (inclusive) that root alone cannot control, or
/// `None` when the whole chain is protected.
fn unsafe_prefix(dir: &Path) -> Option<PathBuf> {
    let mut current = dir.canonicalize().ok()?;
    loop {
        if !crate::install::is_protected_dir(&current) {
            return Some(current);
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// Decide what one candidate copy of the client means.
///
/// Split out from the filesystem walk so every branch is testable without root
/// and without depending on what happens to be installed on the host.
fn classify_candidate(
    candidate: &Path,
    installed: &Path,
    installed_hash: Option<&str>,
    candidate_hash: Option<&str>,
    wins: bool,
    unsafe_dir: Option<&Path>,
) -> Option<Finding> {
    if candidate == installed {
        return None;
    }
    let identical = candidate_hash.is_some() && candidate_hash == installed_hash;
    match (wins, unsafe_dir) {
        (true, _) => Some(finding(
            "CLIENT_SHADOWED",
            Some(candidate),
            format!(
                "this resolves ahead of the installed client {} in the caller's PATH{}",
                installed.display(),
                if identical {
                    "; its bytes match today, but updates land on the installed path only"
                } else {
                    " and its bytes differ"
                }
            ),
        )),
        (false, Some(dir)) => Some(finding(
            "CLIENT_SHADOWED",
            Some(candidate),
            format!(
                "a copy of the client is reachable through {}, which is not root-owned or is \
                 group/world-writable; anyone who can write there chooses what the operator runs",
                dir.display()
            ),
        )),
        (false, None) if identical => Some(finding(
            "CLIENT_DUPLICATE",
            Some(candidate),
            format!(
                "byte-identical copy of the installed client {}; harmless now, but it will not \
                 be updated and PATH order decides which one runs",
                installed.display()
            ),
        )),
        (false, None) => Some(finding(
            "CLIENT_SHADOWED",
            Some(candidate),
            format!(
                "a different `sudo-secretspec` is on the executable search path; the installed \
                 client is {}",
                installed.display()
            ),
        )),
    }
}

/// Report any `sudo-secretspec` outside the installed client path.
///
/// `doctor` otherwise verifies the installation and never asks whether those
/// are the paths that would actually run.
fn check_client_shadowing(
    layout: &Layout,
    dirs: &[PathBuf],
    winner: Option<&Path>,
    findings: &mut Vec<Finding>,
) {
    let Some(name) = layout.client.file_name() else {
        return;
    };
    let installed_real = layout.client.canonicalize().ok();
    let installed_hash = sha256_file(&layout.client);

    let mut reported = std::collections::HashSet::new();
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.symlink_metadata().is_err() {
            continue;
        }
        // A symlink deliberately pointing at the installed client is the same
        // file, not a shadow.
        if matches!(candidate.canonicalize(), Ok(real) if Some(&real) == installed_real.as_ref()) {
            continue;
        }
        if !reported.insert(candidate.clone()) {
            continue;
        }
        // Hash only regular files: opening a fifo left in a caller-supplied
        // directory would block this process as root.
        let candidate_hash = match fs::metadata(&candidate) {
            Ok(meta) if meta.is_file() => sha256_file(&candidate),
            _ => None,
        };
        if let Some(f) = classify_candidate(
            &candidate,
            &layout.client,
            installed_hash.as_deref(),
            candidate_hash.as_deref(),
            winner == Some(candidate.as_path()),
            unsafe_prefix(dir).as_deref(),
        ) {
            findings.push(f);
        }
    }
}

/// Inspect the live installation described by `layout`.
///
/// This never repairs and never opens secret-bearing file contents for parsing.
pub fn inspect(layout: &Layout, opts: &InspectOptions) -> Report {
    let mut findings = Vec::new();

    // Config file itself.
    check_protected_file(&layout.config, "root", "wheel", 0o444, &mut findings);
    check_ancestor_chain(&layout.config, &mut findings);

    // Installed binaries / shared assets.
    for (path, mode) in [
        (&layout.client, 0o755),
        (&layout.broker, 0o755),
        (&layout.engine, 0o755),
        (&layout.retired, 0o444),
        (&layout.guidance, 0o444),
        (&layout.declarations, 0o444),
        (&layout.source_manifest, 0o444),
        (&layout.sudoers, 0o440),
    ] {
        check_protected_file(path, "root", "wheel", mode, &mut findings);
        check_ancestor_chain(path, &mut findings);
    }

    // Which binary would actually run, not just whether the installed one is intact.
    let caller_path = opts.caller_path.as_deref();
    let winner = layout
        .client
        .file_name()
        .and_then(|name| path_winner(caller_path, name));
    check_client_shadowing(
        layout,
        &search_dirs(caller_path),
        winner.as_deref(),
        &mut findings,
    );

    // Source manifest hashes, when present.
    if layout.source_manifest.is_file() {
        for (expected, path) in parse_manifest(&layout.source_manifest) {
            match sha256_file(&path) {
                Some(actual) if actual == expected => {}
                Some(_) => findings.push(finding(
                    "INSTALLED_HASH_MISMATCH",
                    Some(&path),
                    "installed artifact hash differs from release manifest",
                )),
                None => findings.push(finding(
                    "INSTALLED_FILE_MISSING",
                    Some(&path),
                    "manifest lists a missing installed artifact",
                )),
            }
        }
    }

    // Sudoers syntax.
    //
    // Captured, not inherited: `visudo -c` writes "<path>: parsed OK" to
    // stdout, and letting that through put a non-JSON line ahead of the report
    // in `doctor --json`. Anything consuming the report as JSON — which is what
    // AI-GUIDANCE tells automation to do — failed to parse it.
    if layout.sudoers.exists() {
        let checked = Command::new("/usr/sbin/visudo")
            .args(["-c", "-f"])
            .arg(&layout.sudoers)
            .output();
        match checked {
            Ok(out) if out.status.success() => {}
            _ => findings.push(finding(
                "SUDOERS_INVALID",
                Some(&layout.sudoers),
                "visudo rejected the sudoers policy",
            )),
        }
    }

    // Vault metadata and allowlisted runtime entries only.
    if layout.vault.is_symlink() {
        findings.push(finding(
            "VAULT_SYMLINK",
            Some(&layout.vault),
            "vault must not be a symlink",
        ));
    } else {
        check_protected_file(
            &layout.vault,
            &layout.service_user,
            &layout.service_group,
            0o700,
            &mut findings,
        );
    }

    // Resolve public /var spelling through the macOS private chain.
    match layout.vault.canonicalize() {
        Ok(resolved) if resolved != layout.vault_realpath => findings.push(finding(
            "VAULT_REALPATH_MISMATCH",
            Some(&layout.vault),
            format!(
                "vault resolves to {}, expected {}",
                resolved.display(),
                layout.vault_realpath.display()
            ),
        )),
        Ok(resolved) => check_ancestor_chain(&resolved, &mut findings),
        Err(_) => findings.push(finding(
            "VAULT_UNRESOLVABLE",
            Some(&layout.vault),
            "vault path cannot be resolved",
        )),
    }

    let allowed = [
        "secretspec.toml",
        ".env",
        "broker-audit.sqlite3",
        "broker-audit.sqlite3-wal",
        "broker-audit.sqlite3-shm",
        "broker-audit.sqlite3-journal",
    ];
    if let Ok(entries) = fs::read_dir(&layout.vault) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let path = entry.path();
            if name.contains(".rollback.") {
                findings.push(finding(
                    "PENDING_ROLLBACK",
                    Some(&path),
                    "pending rollback artifact requires operator review",
                ));
                // A transient mutation artifact is reported by its own code and
                // deliberately not also judged by the steady-state metadata
                // rule. `fs::copy` does not carry ownership, so a backup left
                // behind by an older broker is root-owned; letting that raise a
                // non-advisory METADATA_MISMATCH would turn any crashed
                // mutation into a hard `doctor` failure, wedging every agent
                // told to treat one as a stop.
                //
                // A symlink here is still refused. That is not drift — it is a
                // redirection of a path `Mutation::restore` copies back over.
                if path.is_symlink() {
                    findings.push(finding(
                        "SYMLINK_FORBIDDEN",
                        Some(&path),
                        "path must not be a symlink",
                    ));
                }
                continue;
            } else if matches!(name.as_ref(), ".local" | ".ansible" | ".cache") {
                findings.push(finding(
                    "LEGACY_VAULT_CLUTTER",
                    Some(&path),
                    "non-secret tool state under the vault; safe to remove after review",
                ));
            } else if !allowed.iter().any(|a| *a == name) {
                findings.push(finding(
                    "UNEXPECTED_RUNTIME_ENTRY",
                    Some(&path),
                    "vault contains a file outside the runtime allowlist",
                ));
            }
            if path.is_file() || path.is_symlink() {
                check_protected_file(
                    &path,
                    &layout.service_user,
                    &layout.service_group,
                    0o600,
                    &mut findings,
                );
            }
        }
    }

    // Audit integrity via library (no secret contents).
    if let Err(e) = audit::verify(&layout.vault, None) {
        findings.push(finding(
            "AUDIT_VERIFY_FAILED",
            Some(&layout.vault.join(audit::DB_NAME)),
            format!("audit verification failed: {e}"),
        ));
    }

    Report::from_findings(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn write_cfg(dir: &Path) -> PathBuf {
        let path = dir.join("sudo-secretspec.toml");
        fs::write(
            &path,
            r#"
engine = "/usr/local/libexec/sudo-secretspec"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn load_config_builds_single_binary_layout() {
        let tmp = tempdir().unwrap();
        let path = write_cfg(tmp.path());
        let layout = load_config(&path).unwrap();
        assert_eq!(
            layout.broker,
            PathBuf::from("/usr/local/libexec/sudo-secretspec")
        );
        assert_eq!(
            layout.client,
            PathBuf::from("/usr/local/bin/sudo-secretspec")
        );
        assert_eq!(layout.vault, PathBuf::from("/var/db/sudo-secretspec"));
    }

    #[test]
    fn advisory_findings_do_not_fail_the_report() {
        // Agents are instructed to treat a drift failure as a hard stop, so
        // vault clutter and a leftover mutation backup must be reported without
        // clearing `ok` — otherwise every automated caller stalls permanently.
        let report = Report::from_findings(vec![
            finding("LEGACY_VAULT_CLUTTER", None, "tool state"),
            finding("PENDING_ROLLBACK", None, "leftover backup"),
        ]);
        assert!(report.ok, "{:?}", report.findings);
        assert!(report.findings.iter().all(|f| f.advisory));
    }

    #[test]
    fn non_advisory_findings_still_fail_the_report() {
        let report = Report::from_findings(vec![
            finding("LEGACY_VAULT_CLUTTER", None, "tool state"),
            finding("METADATA_MISMATCH", None, "wrong owner"),
        ]);
        assert!(!report.ok);
    }

    #[test]
    fn an_empty_report_is_ok() {
        assert!(Report::from_findings(vec![]).ok);
    }

    #[test]
    fn unexpected_runtime_entry_is_reported() {
        let tmp = tempdir().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::set_permissions(&vault, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(vault.join("secretspec.toml"), "x = 1").unwrap();
        fs::write(vault.join(".env"), "").unwrap();
        fs::write(vault.join("evil.txt"), "nope").unwrap();

        let layout = Layout {
            config: tmp.path().join("missing.toml"),
            vault: vault.clone(),
            vault_realpath: vault.clone(),
            service_user: owner_name(unsafe { libc::getuid() }),
            service_group: group_name(unsafe { libc::getgid() }),
            declarations: tmp.path().join("decl.toml"),
            engine: tmp.path().join("engine"),
            audit: tmp.path().join("audit"),
            broker: tmp.path().join("broker"),
            client: tmp.path().join("client"),
            checker: tmp.path().join("checker"),
            retired: tmp.path().join("retired"),
            guidance: tmp.path().join("guidance"),
            sudoers: tmp.path().join("sudoers"),
            source_manifest: tmp.path().join("MANIFEST.sha256"),
        };
        let report = inspect(&layout, &InspectOptions::default());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == "UNEXPECTED_RUNTIME_ENTRY"),
            "{:?}",
            report.findings
        );
    }

    /// A `Layout` whose vault is `vault` and whose every other path is absent.
    /// Only the vault-scan findings are meaningful for callers of this.
    fn layout_for_vault(tmp: &Path, vault: &Path) -> Layout {
        Layout {
            config: tmp.join("missing.toml"),
            vault: vault.to_path_buf(),
            vault_realpath: vault.to_path_buf(),
            service_user: owner_name(unsafe { libc::getuid() }),
            service_group: group_name(unsafe { libc::getgid() }),
            declarations: tmp.join("decl.toml"),
            engine: tmp.join("engine"),
            audit: tmp.join("audit"),
            broker: tmp.join("broker"),
            client: tmp.join("client"),
            checker: tmp.join("checker"),
            retired: tmp.join("retired"),
            guidance: tmp.join("guidance"),
            sudoers: tmp.join("sudoers"),
            source_manifest: tmp.join("MANIFEST.sha256"),
        }
    }

    fn findings_for<'a>(report: &'a Report, path: &Path) -> Vec<&'a str> {
        report
            .findings
            .iter()
            .filter(|f| f.path.as_deref() == Some(path.to_string_lossy().as_ref()))
            .map(|f| f.code.as_str())
            .collect()
    }

    #[test]
    fn a_pending_rollback_backup_is_not_also_judged_by_the_steady_state_rule() {
        // `fs::copy` preserves mode but not ownership, so a backup made by the
        // root broker lands root-owned inside a service-user vault. The
        // per-entry metadata rule then turned the deliberately-advisory
        // PENDING_ROLLBACK into a non-advisory METADATA_MISMATCH, failing
        // `doctor` — and every agent is told to treat that as a hard stop, so a
        // single crashed mutation wedged the host until someone cleaned up by
        // hand. Ownership cannot be forged in an unprivileged test; a differing
        // mode trips the very same rule on the very same entry.
        let tmp = tempdir().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::set_permissions(&vault, fs::Permissions::from_mode(0o700)).unwrap();

        let backup = vault.join(".secretspec.toml.rollback.0f9c1e6a");
        fs::write(&backup, "manifest backup").unwrap();
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o644)).unwrap();

        // Control: an entry with the same wrong mode that is *not* a rollback
        // artifact must still be judged, or this test would pass vacuously.
        let control = vault.join("secretspec.toml");
        fs::write(&control, "x = 1").unwrap();
        fs::set_permissions(&control, fs::Permissions::from_mode(0o644)).unwrap();

        let report = inspect(&layout_for_vault(tmp.path(), &vault), &Default::default());

        assert_eq!(
            findings_for(&report, &backup),
            ["PENDING_ROLLBACK"],
            "the backup must be reported by its own code and nothing else"
        );
        assert!(
            findings_for(&report, &control).contains(&"METADATA_MISMATCH"),
            "control entry must still be judged: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_symlinked_rollback_backup_is_still_refused() {
        // Skipping the metadata rule must not also skip this: `restore` copies
        // a backup back over the live manifest, so a symlink in that position
        // redirects what a root process is about to read.
        let tmp = tempdir().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::set_permissions(&vault, fs::Permissions::from_mode(0o700)).unwrap();

        let target = tmp.path().join("elsewhere");
        fs::write(&target, "attacker controlled").unwrap();
        let backup = vault.join(".secretspec.env.rollback.0f9c1e6a");
        std::os::unix::fs::symlink(&target, &backup).unwrap();

        let report = inspect(&layout_for_vault(tmp.path(), &vault), &Default::default());
        let codes = findings_for(&report, &backup);
        assert!(codes.contains(&"SYMLINK_FORBIDDEN"), "{codes:?}");
        assert!(!report.ok, "a symlinked backup must fail the report");
    }

    const INSTALLED: &str = "/usr/local/bin/sudo-secretspec";

    fn classify(
        candidate: &str,
        candidate_hash: Option<&str>,
        wins: bool,
        unsafe_dir: Option<&str>,
    ) -> Option<Finding> {
        classify_candidate(
            Path::new(candidate),
            Path::new(INSTALLED),
            Some("aaaa"),
            candidate_hash,
            wins,
            unsafe_dir.map(Path::new),
        )
    }

    #[test]
    fn the_installed_client_is_not_its_own_shadow() {
        assert!(classify(INSTALLED, Some("aaaa"), true, None).is_none());
    }

    #[test]
    fn a_copy_that_wins_the_callers_path_fails_even_when_identical() {
        // Identical bytes today are not reassurance: `install` only ever writes
        // the installed path, so the next release makes them diverge silently.
        let f = classify(
            "/opt/homebrew/bin/sudo-secretspec",
            Some("aaaa"),
            true,
            None,
        )
        .unwrap();
        assert_eq!(f.code, "CLIENT_SHADOWED");
        assert!(!f.advisory);
    }

    #[test]
    fn a_copy_under_a_writable_prefix_fails_even_when_it_loses_path_order() {
        // Losing today is an accident of PATH order; anyone who can write the
        // directory can still swap the bytes.
        let f = classify(
            "/opt/homebrew/bin/sudo-secretspec",
            Some("aaaa"),
            false,
            Some("/opt/homebrew"),
        )
        .unwrap();
        assert_eq!(f.code, "CLIENT_SHADOWED");
        assert!(!f.advisory);
        assert!(f.detail.contains("/opt/homebrew"), "{}", f.detail);
    }

    #[test]
    fn differing_bytes_fail_wherever_they_sit() {
        let f = classify("/usr/local/sbin/sudo-secretspec", Some("bbbb"), false, None).unwrap();
        assert_eq!(f.code, "CLIENT_SHADOWED");
        assert!(!f.advisory);
    }

    #[test]
    fn an_unreadable_or_non_regular_candidate_is_not_treated_as_identical() {
        let f = classify("/usr/local/sbin/sudo-secretspec", None, false, None).unwrap();
        assert_eq!(f.code, "CLIENT_SHADOWED");
    }

    #[test]
    fn an_identical_losing_copy_under_a_root_only_prefix_is_advisory() {
        let f = classify("/usr/local/sbin/sudo-secretspec", Some("aaaa"), false, None).unwrap();
        assert_eq!(f.code, "CLIENT_DUPLICATE");
        assert!(f.advisory, "must not wedge automated callers");
    }

    #[test]
    fn caller_path_can_only_widen_the_scan() {
        let dirs = search_dirs(Some(OsStr::new("/opt/mine/bin:relative/bin:/usr/bin")));
        for fixed in SEARCH_DIRS {
            assert!(
                dirs.iter().any(|d| d == Path::new(fixed)),
                "caller PATH must not be able to drop {fixed}"
            );
        }
        assert!(dirs.iter().any(|d| d == Path::new("/opt/mine/bin")));
        // Relative entries would resolve against a working directory the caller
        // also controls.
        assert!(!dirs.iter().any(|d| d.as_os_str() == "relative/bin"));
        // /usr/bin is fixed and repeated in the caller value; it appears once.
        assert_eq!(
            dirs.iter().filter(|d| *d == Path::new("/usr/bin")).count(),
            1
        );
    }

    #[test]
    fn no_caller_path_means_the_fixed_list_only() {
        assert_eq!(search_dirs(None).len(), SEARCH_DIRS.len());
    }

    #[test]
    fn path_winner_picks_the_first_existing_entry() {
        let tmp = tempdir().unwrap();
        let empty = tmp.path().join("empty");
        let real = tmp.path().join("real");
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("sudo-secretspec"), b"x").unwrap();

        let path = std::env::join_paths([&empty, &real]).unwrap();
        assert_eq!(
            path_winner(Some(&path), OsStr::new("sudo-secretspec")),
            Some(real.join("sudo-secretspec"))
        );
        assert_eq!(path_winner(None, OsStr::new("sudo-secretspec")), None);
    }

    #[test]
    fn shadowing_walks_the_supplied_directories() {
        let tmp = tempdir().unwrap();
        let installed_dir = tmp.path().join("bin");
        let other = tmp.path().join("other");
        fs::create_dir_all(&installed_dir).unwrap();
        fs::create_dir_all(&other).unwrap();
        let installed = installed_dir.join("sudo-secretspec");
        fs::write(&installed, b"real").unwrap();
        fs::write(other.join("sudo-secretspec"), b"different").unwrap();

        let mut layout = layout_for(tmp.path());
        layout.client = installed.clone();

        let mut findings = Vec::new();
        check_client_shadowing(
            &layout,
            &[installed_dir.clone(), other.clone()],
            None,
            &mut findings,
        );
        // The installed directory contributes nothing; the other one does.
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].code, "CLIENT_SHADOWED");
        assert_eq!(
            findings[0].path.as_deref(),
            Some(other.join("sudo-secretspec").display().to_string().as_str())
        );
    }

    #[test]
    fn a_symlink_to_the_installed_client_is_not_a_shadow() {
        let tmp = tempdir().unwrap();
        let installed_dir = tmp.path().join("bin");
        let other = tmp.path().join("other");
        fs::create_dir_all(&installed_dir).unwrap();
        fs::create_dir_all(&other).unwrap();
        let installed = installed_dir.join("sudo-secretspec");
        fs::write(&installed, b"real").unwrap();
        std::os::unix::fs::symlink(&installed, other.join("sudo-secretspec")).unwrap();

        let mut layout = layout_for(tmp.path());
        layout.client = installed;

        let mut findings = Vec::new();
        check_client_shadowing(&layout, &[installed_dir, other], None, &mut findings);
        assert!(findings.is_empty(), "{findings:?}");
    }

    fn layout_for(root: &Path) -> Layout {
        Layout {
            config: root.join("config.toml"),
            vault: root.join("vault"),
            vault_realpath: root.join("vault"),
            service_user: owner_name(unsafe { libc::getuid() }),
            service_group: group_name(unsafe { libc::getgid() }),
            declarations: root.join("decl.toml"),
            engine: root.join("engine"),
            audit: root.join("audit"),
            broker: root.join("broker"),
            client: root.join("client"),
            checker: root.join("checker"),
            retired: root.join("retired"),
            guidance: root.join("guidance"),
            sudoers: root.join("sudoers"),
            source_manifest: root.join("MANIFEST.sha256"),
        }
    }
}
