//! Artifact rollback for sudo-secretspec privileged installs.
//!
//! Restores previous installed binaries/config/sudoers from a protected
//! snapshot directory. Never touches vault secret values.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RollbackError {
    #[error("{0}")]
    Denied(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

fn require_root() -> Result<(), RollbackError> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(RollbackError::Denied("rollback requires root".into()));
    }
    Ok(())
}

/// Restore installed artifacts from a protected snapshot directory.
///
/// Expected layout:
/// - `MANIFEST.sha256` listing `hash  absolute-path` rows for prior artifacts
/// - optional numbered backups or direct prior file copies named by sha path basenames
///
/// Current installer creates snapshot dirs as placeholders; a complete snapshot
/// writer can store prior bytes beside the manifest. This restore verifies the
/// snapshot directory metadata and reapplies any `*.prior` files found as:
///   `<index>.prior` + `<index>.path` containing the destination absolute path.
pub fn run(snapshot: &Path) -> Result<(), RollbackError> {
    require_root()?;
    let snapshot = fs::canonicalize(snapshot)
        .map_err(|e| RollbackError::Denied(format!("invalid snapshot path: {e}")))?;
    let prefix = PathBuf::from("/usr/local/libexec/sudo-secretspec-rollback-");
    if !snapshot
        .to_string_lossy()
        .starts_with(prefix.to_string_lossy().as_ref())
    {
        return Err(RollbackError::Denied(
            "rollback snapshot is outside the protected rollback directory".into(),
        ));
    }
    if snapshot.is_symlink() {
        return Err(RollbackError::Denied(
            "snapshot must not be a symlink".into(),
        ));
    }
    let meta = fs::metadata(&snapshot)?;
    if meta.permissions().mode() & 0o777 != 0o700 {
        return Err(RollbackError::Denied(
            "snapshot directory mode must be 0700".into(),
        ));
    }

    // Prefer an explicit restore program if the snapshot contains one.
    let restore_bin = snapshot.join("restore");
    if restore_bin.is_file() {
        let status = Command::new(&restore_bin).status()?;
        if status.success() {
            println!("sudo-secretspec artifacts restored; runtime vault preserved");
            return Ok(());
        }
        return Err(RollbackError::Denied(
            "snapshot restore program failed".into(),
        ));
    }

    // Generic path-pair restore: N.path + N.prior
    let mut pairs = Vec::new();
    for entry in fs::read_dir(&snapshot)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(idx) = name.strip_suffix(".path") {
            let prior = snapshot.join(format!("{idx}.prior"));
            if prior.is_file() {
                let dest = PathBuf::from(fs::read_to_string(entry.path())?.trim());
                pairs.push((prior, dest));
            }
        }
    }
    if pairs.is_empty() {
        return Err(RollbackError::Denied(
            "snapshot contains no restorable prior artifacts".into(),
        ));
    }

    for (prior, dest) in pairs.iter().rev() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = dest.with_extension(format!("restore.{}", std::process::id()));
        fs::copy(prior, &tmp)?;
        fs::rename(&tmp, dest)?;
    }

    if Path::new("/private/etc/sudoers.d/sudo-secretspec").exists() {
        let status = Command::new("/usr/sbin/visudo")
            .args(["-c", "-f", "/private/etc/sudoers.d/sudo-secretspec"])
            .status()?;
        if !status.success() {
            return Err(RollbackError::Denied(
                "restored sudoers failed visudo validation".into(),
            ));
        }
    }

    println!("sudo-secretspec artifacts restored; runtime vault preserved");
    Ok(())
}
