//! Metadata-only, non-repairing drift validation for sudo-secretspec.
//!
//! Never reads secret values. Never mutates state.

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Report {
    pub ok: bool,
    pub findings: Vec<Finding>,
}

impl Report {
    fn from_findings(findings: Vec<Finding>) -> Self {
        Self {
            ok: findings.is_empty(),
            findings,
        }
    }
}

fn finding(code: &str, path: Option<&Path>, detail: impl Into<String>) -> Finding {
    Finding {
        code: code.into(),
        detail: detail.into(),
        path: path.map(|p| p.display().to_string()),
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

fn parse_manifest(path: &Path) -> Vec<(String, PathBuf)> {
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

/// Inspect the live installation described by `layout`.
///
/// This never repairs and never opens secret-bearing file contents for parsing.
pub fn inspect(layout: &Layout) -> Report {
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
    if layout.sudoers.exists() {
        let status = Command::new("/usr/sbin/visudo")
            .args(["-c", "-f"])
            .arg(&layout.sudoers)
            .status();
        match status {
            Ok(s) if s.success() => {}
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
        let report = inspect(&layout);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == "UNEXPECTED_RUNTIME_ENTRY"),
            "{:?}",
            report.findings
        );
    }
}
