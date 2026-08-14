//! Privileged installer for the sudo-secretspec boundary.
//!
//! Fresh installs can create the dedicated service identity and vault.
//! Existing protected stores must be adopted explicitly. Dry-run validates
//! without mutating.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "/usr/local";
const CONFIG_PATH: &str = "/usr/local/etc/sudo-secretspec.toml";
const SUDOERS_PATH: &str = "/private/etc/sudoers.d/sudo-secretspec";
/// Any file installed under here is a sudo policy and must be validated before
/// it is allowed to take effect.
pub(crate) const SUDOERS_DIR: &str = "/private/etc/sudoers.d";
const DEFAULT_VAULT: &str = "/var/db/sudo-secretspec";
const DEFAULT_USER: &str = "_sudo_secretspec";
const DEFAULT_GROUP: &str = "_sudo_secretspec";
/// Directory-name prefix for rollback snapshots under `<PREFIX>/libexec`.
/// `rollback::run` refuses any snapshot path outside this namespace.
pub(crate) const SNAPSHOT_PREFIX: &str = "sudo-secretspec-rollback-";
/// Restorable snapshots retained after a successful install. Every install
/// captures one, so without a bound they accumulate for the life of the host.
const SNAPSHOT_KEEP: usize = 3;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("{0}")]
    Denied(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub declarations: PathBuf,
    pub dry_run: bool,
    pub adopt_existing: bool,
    pub vault: PathBuf,
    pub service_user: String,
    pub service_group: String,
    pub operator: String,
    pub source_root: Option<PathBuf>,
}

impl InstallRequest {
    pub fn from_cli(declarations: PathBuf, dry_run: bool, adopt_existing: bool) -> Self {
        let operator = std::env::var("SUDO_USER")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "root".into());
        Self {
            declarations,
            dry_run,
            adopt_existing,
            vault: PathBuf::from(DEFAULT_VAULT),
            service_user: DEFAULT_USER.into(),
            service_group: DEFAULT_GROUP.into(),
            operator,
            source_root: None,
        }
    }
}

/// Every absolute path the installer owns, with the mode it must be installed
/// at.
///
/// This is the single source of truth shared by three callers: the installer
/// writes these modes, `capture_snapshot` preserves exactly these paths, and
/// `rollback` refuses to write anywhere outside this set. Rollback restoring a
/// mode from anywhere else would be a way to smuggle in a different one — the
/// sudoers policy at `0440` and the client binary at `0755` both matter.
pub fn installed_artifacts() -> Vec<(PathBuf, u32)> {
    let prefix = PathBuf::from(PREFIX);
    let share = prefix.join("share/sudo-secretspec");
    vec![
        (prefix.join("bin/sudo-secretspec"), 0o755),
        (prefix.join("libexec/sudo-secretspec"), 0o755),
        (share.join("secretspec.toml"), 0o444),
        (share.join("sudo-secretspec-retired.toml"), 0o444),
        (share.join("AI-GUIDANCE.md"), 0o444),
        (share.join("MANIFEST.sha256"), 0o444),
        (PathBuf::from(CONFIG_PATH), 0o444),
        (PathBuf::from(SUDOERS_PATH), 0o440),
    ]
}

/// Installed mode for `path`, or `None` when it is not an owned artifact.
pub fn artifact_mode(path: &Path) -> Option<u32> {
    installed_artifacts()
        .into_iter()
        .find(|(p, _)| p == path)
        .map(|(_, mode)| mode)
}

fn require_mode(path: &Path) -> Result<u32, InstallError> {
    artifact_mode(path).ok_or_else(|| {
        InstallError::Denied(format!("not an owned install artifact: {}", path.display()))
    })
}

/// Copy the current bytes of every installed artifact into `snapshot` so a
/// later `rollback` can restore exactly this state.
///
/// Each prior file is recorded as `<index>.prior` alongside `<index>.path`
/// holding its absolute destination, plus a `MANIFEST.sha256` binding every
/// destination to the hash of its captured bytes. Rollback verifies that
/// manifest, so a snapshot cannot be edited into a delivery vehicle for other
/// content. On a first install there is nothing to capture and the manifest is
/// written empty.
fn capture_snapshot(snapshot: &Path) -> Result<usize, InstallError> {
    fs::create_dir_all(snapshot)?;
    let mut perms = fs::metadata(snapshot)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(snapshot, perms)?;

    let mut manifest = String::new();
    let mut captured = 0usize;
    for (index, (dest, _mode)) in installed_artifacts().iter().enumerate() {
        let meta = match fs::symlink_metadata(dest) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            return Err(InstallError::Denied(format!(
                "refusing to snapshot a non-regular installed path: {}",
                dest.display()
            )));
        }
        let prior = snapshot.join(format!("{index}.prior"));
        fs::copy(dest, &prior)?;
        let mut p = fs::metadata(&prior)?.permissions();
        p.set_mode(0o600);
        fs::set_permissions(&prior, p)?;
        write_bytes(
            &snapshot.join(format!("{index}.path")),
            dest.display().to_string().as_bytes(),
            0o600,
        )?;
        manifest.push_str(&format!("{}  {}\n", sha256_file(&prior)?, dest.display()));
        captured += 1;
    }
    write_bytes(
        &snapshot.join("MANIFEST.sha256"),
        manifest.as_bytes(),
        0o600,
    )?;
    Ok(captured)
}

/// One rollback snapshot directory as it appears on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    /// Unix seconds parsed from the directory name.
    pub stamp: u64,
    /// False when the directory holds no `.prior` file. `plan_restore` rejects
    /// such a snapshot outright ("no restorable prior artifacts"), so it can
    /// never be used for anything — a first install produces one every time.
    pub restorable: bool,
}

/// Enumerate rollback snapshots directly under `dir`.
///
/// Only directories whose name carries [`SNAPSHOT_PREFIX`] followed by a
/// numeric stamp are reported, so an unrelated neighbour in libexec can never
/// become a pruning candidate.
pub fn list_snapshots(dir: &Path) -> Vec<Snapshot> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stamp) = name.strip_prefix(SNAPSHOT_PREFIX) else {
            continue;
        };
        let Ok(stamp) = stamp.parse::<u64>() else {
            continue;
        };
        let path = entry.path();
        if path.is_symlink() || !path.is_dir() {
            continue;
        }
        let restorable = fs::read_dir(&path)
            .map(|mut e| {
                e.any(|f| {
                    f.map(|f| f.file_name().to_string_lossy().ends_with(".prior"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        found.push(Snapshot {
            path,
            stamp,
            restorable,
        });
    }
    found
}

/// Choose which snapshots to delete: every unrestorable one, plus restorable
/// ones older than the newest `keep`.
///
/// Separated from [`prune_snapshots`] so the retention decision can be tested
/// without root, the same way `rollback::plan_restore` separates its trust
/// decisions from the filesystem work they authorise.
pub fn plan_prune(snapshots: &[Snapshot], keep: usize) -> Vec<PathBuf> {
    let mut restorable: Vec<&Snapshot> = snapshots.iter().filter(|s| s.restorable).collect();
    // Newest first, so the tail past `keep` is what ages out.
    restorable.sort_by(|a, b| b.stamp.cmp(&a.stamp));

    let mut doomed: Vec<PathBuf> = snapshots
        .iter()
        .filter(|s| !s.restorable)
        .map(|s| s.path.clone())
        .collect();
    doomed.extend(restorable.iter().skip(keep).map(|s| s.path.clone()));
    doomed.sort();
    doomed
}

/// Delete the snapshots [`plan_prune`] selects.
///
/// Each candidate is re-checked against the same guards `rollback::run`
/// applies before it trusts a snapshot — root-owned, mode 0700, not a symlink.
/// A directory failing any of them is left alone rather than removed: it is
/// not ours, and deleting it as root on a guess is the worse error.
/// Returns the number removed. Never fails an install: pruning is hygiene.
fn prune_snapshots(dir: &Path, keep: usize) -> usize {
    let mut removed = 0;
    for path in plan_prune(&list_snapshots(dir), keep) {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.uid() != 0 || meta.permissions().mode() & 0o777 != 0o700 {
            continue;
        }
        if fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn require_root() -> Result<(), InstallError> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(InstallError::Denied(
            "must run as root (Touch ID/sudo is expected, including dry-run)".into(),
        ));
    }
    Ok(())
}

fn resolve_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn sha256_file(path: &Path) -> Result<String, InstallError> {
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn install_file(src: &Path, dst: &Path, mode: u32) -> Result<(), InstallError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension(format!("new.{}", std::process::id()));
    fs::copy(src, &tmp)?;
    let mut perms = fs::metadata(&tmp)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(&tmp, perms)?;
    // Best-effort ownership; already root when installer is root.
    let _ = Command::new("/usr/sbin/chown")
        .args(["root:wheel"])
        .arg(&tmp)
        .status();
    fs::rename(&tmp, dst)?;
    Ok(())
}

fn write_bytes(dst: &Path, bytes: &[u8], mode: u32) -> Result<(), InstallError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension(format!("new.{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
    }
    let mut perms = fs::metadata(&tmp)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(&tmp, perms)?;
    let _ = Command::new("/usr/sbin/chown")
        .args(["root:wheel"])
        .arg(&tmp)
        .status();
    fs::rename(&tmp, dst)?;
    Ok(())
}

/// Write a candidate sudoers policy beside `dst` and validate it, returning the
/// staged path only if `visudo` accepts it.
///
/// A syntactically invalid file under `sudoers.d` makes sudo refuse to run at
/// all, which would strand the operator with no way to elevate — including no
/// way to re-run this installer and repair it. So the policy is never written
/// to its live name until it has parsed cleanly.
///
/// The staging name deliberately contains dots. sudo ignores files in
/// `sudoers.d` whose names contain a `.` or end in `~`, so even if this process
/// dies between writing and renaming, the staged file is inert.
pub fn stage_sudoers(
    dst: &Path,
    text: &str,
    mode: u32,
) -> Result<std::path::PathBuf, InstallError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = dst.with_extension(format!("staged.{}", std::process::id()));
    {
        let mut f = fs::File::create(&staged)?;
        f.write_all(text.as_bytes())?;
    }
    let mut perms = fs::metadata(&staged)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(&staged, perms)?;
    let _ = Command::new("/usr/sbin/chown")
        .args(["root:wheel"])
        .arg(&staged)
        .status();

    let accepted = Command::new("/usr/sbin/visudo")
        .args(["-c", "-f"])
        .arg(&staged)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !accepted {
        let _ = fs::remove_file(&staged);
        return Err(InstallError::Denied(
            "generated sudoers policy failed visudo validation; the existing policy was left \
             untouched"
                .into(),
        ));
    }
    Ok(staged)
}

/// Install the sudoers policy so that a bad policy can never take effect.
///
/// Staged and validated in isolation first, then renamed into place, then the
/// combined configuration is re-checked with a bare `visudo -c`. If that final
/// check fails the previous policy is put back — or the file removed when there
/// was no previous policy — before returning an error.
fn install_sudoers(dst: &Path, text: &str, mode: u32) -> Result<(), InstallError> {
    let previous = fs::read(dst).ok();
    let staged = stage_sudoers(dst, text, mode)?;
    fs::rename(&staged, dst)?;

    // Valid in isolation is not the same as valid in combination; re-check the
    // whole configuration now that the file is live.
    //
    // A failure here is only ours to act on if *our* file is the one at fault.
    // Unrelated policies in sudoers.d — another package shipping a file with
    // the wrong mode, say — must not make this installer unusable, so re-check
    // our own file and treat anyone else's problem as a warning.
    let combined_ok = Command::new("/usr/sbin/visudo")
        .arg("-c")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if combined_ok {
        return Ok(());
    }

    let ours_ok = Command::new("/usr/sbin/visudo")
        .args(["-c", "-f"])
        .arg(dst)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ours_ok {
        eprintln!(
            "warning: `visudo -c` reports a problem elsewhere in the sudoers configuration.\n\
             warning: {} itself is valid and has been installed. Review the output above;\n\
             warning: sudo ignores files in sudoers.d with the wrong mode or a dot in the name.",
            dst.display()
        );
        return Ok(());
    }

    match &previous {
        Some(bytes) => {
            let restore = dst.with_extension(format!("restore.{}", std::process::id()));
            fs::write(&restore, bytes)?;
            let mut perms = fs::metadata(&restore)?.permissions();
            perms.set_mode(mode);
            fs::set_permissions(&restore, perms)?;
            let _ = Command::new("/usr/sbin/chown")
                .args(["root:wheel"])
                .arg(&restore)
                .status();
            fs::rename(&restore, dst)?;
        }
        None => {
            let _ = fs::remove_file(dst);
        }
    }
    Err(InstallError::Denied(
        "sudoers configuration failed validation with the new policy in place; the previous \
         policy has been restored"
            .into(),
    ))
}

fn ensure_service_group(name: &str, create: bool) -> Result<u32, InstallError> {
    let output = Command::new("/usr/bin/dscl")
        .args([".", "-read", &format!("/Groups/{name}"), "PrimaryGroupID"])
        .output()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        let gid = text
            .split_whitespace()
            .last()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| InstallError::Denied("cannot parse group id".into()))?;
        return Ok(gid);
    }
    if !create {
        return Err(InstallError::Denied(format!(
            "adopted group missing: {name}"
        )));
    }
    // Find free GID 400-499.
    let mut gid = 499u32;
    while gid >= 400 {
        let probe = Command::new("/usr/bin/dscl")
            .args([
                ".",
                "-search",
                "/Groups",
                "PrimaryGroupID",
                &gid.to_string(),
            ])
            .output()?;
        if !String::from_utf8_lossy(&probe.stdout).contains(char::is_alphanumeric) {
            break;
        }
        gid -= 1;
    }
    if gid < 400 {
        return Err(InstallError::Denied("no free hidden group id".into()));
    }
    for args in [
        vec![".", "-create", &format!("/Groups/{name}")],
        vec![
            ".",
            "-create",
            &format!("/Groups/{name}"),
            "PrimaryGroupID",
            &gid.to_string(),
        ],
        vec![".", "-create", &format!("/Groups/{name}"), "Password", "*"],
    ] {
        let status = Command::new("/usr/bin/dscl").args(&args).status()?;
        if !status.success() {
            return Err(InstallError::Denied(format!(
                "failed to create group {name}"
            )));
        }
    }
    Ok(gid)
}

fn ensure_service_user(name: &str, gid: u32, create: bool) -> Result<(), InstallError> {
    let output = Command::new("/usr/bin/id").arg(name).output()?;
    if output.status.success() {
        return Ok(());
    }
    if !create {
        return Err(InstallError::Denied(format!(
            "adopted user missing: {name}"
        )));
    }
    let mut uid = 499u32;
    while uid >= 400 {
        let probe = Command::new("/usr/bin/dscl")
            .args([".", "-search", "/Users", "UniqueID", &uid.to_string()])
            .output()?;
        if !String::from_utf8_lossy(&probe.stdout).contains(char::is_alphanumeric) {
            break;
        }
        uid -= 1;
    }
    if uid < 400 {
        return Err(InstallError::Denied("no free hidden user id".into()));
    }
    let home = "/var/empty";
    let shell = "/usr/bin/false";
    let path = format!("/Users/{name}");
    for args in [
        vec![".", "-create", &path],
        vec![".", "-create", &path, "UniqueID", &uid.to_string()],
        vec![".", "-create", &path, "PrimaryGroupID", &gid.to_string()],
        vec![".", "-create", &path, "UserShell", shell],
        vec![".", "-create", &path, "NFSHomeDirectory", home],
        vec![".", "-create", &path, "IsHidden", "1"],
        vec![".", "-create", &path, "Password", "*"],
    ] {
        let status = Command::new("/usr/bin/dscl").args(&args).status()?;
        if !status.success() {
            return Err(InstallError::Denied(format!(
                "failed to create user {name}"
            )));
        }
    }
    Ok(())
}

fn validate_protected_ancestors() -> Result<(), InstallError> {
    for dir in [
        "/usr",
        "/usr/local",
        "/private",
        "/private/var",
        "/private/var/db",
        "/private/etc",
        "/private/etc/sudoers.d",
    ] {
        let path = Path::new(dir);
        if !path.is_dir() || path.is_symlink() {
            return Err(InstallError::Denied(format!(
                "unsafe protected directory {dir}"
            )));
        }
        let meta = fs::metadata(path)?;
        if meta.uid() != 0 || (meta.permissions().mode() & 0o022) != 0 {
            return Err(InstallError::Denied(format!(
                "unsafe protected directory metadata {dir}"
            )));
        }
    }
    Ok(())
}

fn config_toml(req: &InstallRequest, vault_real: &Path) -> String {
    format!(
        "engine = \"{prefix}/libexec/sudo-secretspec\"\n\
         audit_helper = \"{prefix}/libexec/sudo-secretspec\"\n\
         vault = \"{vault}\"\n\
         vault_realpath = \"{vault_real}\"\n\
         declarations = \"{prefix}/share/sudo-secretspec/secretspec.toml\"\n\
         service_user = \"{user}\"\n\
         service_group = \"{group}\"\n",
        prefix = PREFIX,
        vault = req.vault.display(),
        vault_real = vault_real.display(),
        user = req.service_user,
        group = req.service_group,
    )
}

/// Sudoers policy for the operator.
///
/// The grant is deliberately per-subcommand. A blanket `sudo-secretspec *`
/// would also cover `install` and `rollback` — the same binary serves as both
/// client and broker — which would let any caller running as the operator
/// reconfigure or restore the boundary with no interactive authentication.
/// Boundary lifecycle must stay behind Touch ID via the public client, so only
/// the mediated broker operations and the read-only doctor are NOPASSWD here.
/// `main` enforces the same restriction inside the binary, because sudoers
/// argument matching alone is easy to get subtly wrong.
fn sudoers_text(operator: &str) -> String {
    format!(
        "Defaults!{prefix}/libexec/sudo-secretspec env_reset,secure_path=/usr/bin:/bin:/usr/sbin:/sbin,umask=0077\n\
         {operator} ALL=(root) NOPASSWD: {prefix}/libexec/sudo-secretspec __broker *\n\
         {operator} ALL=(root) NOPASSWD: {prefix}/libexec/sudo-secretspec doctor\n\
         {operator} ALL=(root) NOPASSWD: {prefix}/libexec/sudo-secretspec doctor *\n",
        prefix = PREFIX,
        operator = operator,
    )
}

fn retired_toml() -> &'static str {
    "# RETIRED SECRETSPEC PATH — NO SECRET VALUES\n\
     # This path is not a credential store. Use /usr/local/bin/sudo-secretspec.\n\
     # Do not create, copy, symlink, regenerate, relocate, or select an alternate\n\
     # manifest or provider.\n"
}

fn guidance_text() -> &'static str {
    // Keep installer self-contained so packaging does not depend on relative
    // source layout after cargo install.
    "# AI and automation contract for sudo-secretspec\n\n\
     Use only /usr/local/bin/sudo-secretspec. Never select manifests, providers,\n\
     profiles, or backing files. Fail closed if the broker is unavailable.\n"
}

/// Run the installer. Returns Ok(()) on success.
pub fn run(req: InstallRequest) -> Result<(), InstallError> {
    require_root()?;
    if !req.declarations.is_file() || req.declarations.is_symlink() {
        return Err(InstallError::Denied(
            "declarations must be a regular file".into(),
        ));
    }
    validate_protected_ancestors()?;

    if req.dry_run {
        if req.adopt_existing {
            // Metadata-only validation of existing identity/vault.
            let id_status = Command::new("/usr/bin/id")
                .arg(&req.service_user)
                .status()
                .map_err(|e| InstallError::Denied(e.to_string()))?;
            if !id_status.success() {
                return Err(InstallError::Denied(format!(
                    "adopted user missing: {}",
                    req.service_user
                )));
            }
            let group_status = Command::new("/usr/bin/dscl")
                .args([".", "-read", &format!("/Groups/{}", req.service_group)])
                .status()
                .map_err(|e| InstallError::Denied(e.to_string()))?;
            if !group_status.success() {
                return Err(InstallError::Denied(format!(
                    "adopted group missing: {}",
                    req.service_group
                )));
            }
            if !req.vault.is_dir() || req.vault.is_symlink() {
                return Err(InstallError::Denied(
                    "adopted vault missing or symlinked".into(),
                ));
            }
            // Root-only metadata checks of vault contents (never open values).
            for runtime in ["secretspec.toml", ".env"] {
                let p = req.vault.join(runtime);
                let meta = fs::symlink_metadata(&p).map_err(|_| {
                    InstallError::Denied(format!(
                        "adopted runtime file missing or unreadable: {}",
                        p.display()
                    ))
                })?;
                if meta.file_type().is_symlink() || !meta.is_file() {
                    return Err(InstallError::Denied(format!(
                        "adopted runtime file missing or symlinked: {}",
                        p.display()
                    )));
                }
            }
            let vault_real = resolve_path(&req.vault);
            if !vault_real.starts_with("/private/var/db/") {
                return Err(InstallError::Denied(
                    "vault resolves outside /private/var/db".into(),
                ));
            }
        } else if Path::new(&req.vault).exists()
            || Command::new("/usr/bin/id")
                .arg(&req.service_user)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        {
            return Err(InstallError::Denied(
                "fresh-install identity or vault already exists; use --adopt-existing".into(),
            ));
        }
        println!("would install sudo-secretspec");
        println!("operator={}", req.operator);
        println!("service={}:{}", req.service_user, req.service_group);
        println!("vault={}", req.vault.display());
        println!("prefix={PREFIX}");
        println!("adopt_existing={}", u8::from(req.adopt_existing));
        return Ok(());
    }

    // Create or adopt service identity.
    let gid = ensure_service_group(&req.service_group, !req.adopt_existing)?;
    ensure_service_user(&req.service_user, gid, !req.adopt_existing)?;

    // Vault.
    let mut created_vault = false;
    if !req.vault.exists() {
        if req.adopt_existing {
            return Err(InstallError::Denied("adopted vault missing".into()));
        }
        fs::create_dir_all(&req.vault)?;
        created_vault = true;
    }
    if req.vault.is_symlink() {
        return Err(InstallError::Denied("vault must not be a symlink".into()));
    }
    if req.adopt_existing {
        // Never rewrite ownership of an adopted vault; only verify metadata.
        let meta = fs::metadata(&req.vault)?;
        if (meta.permissions().mode() & 0o777) != 0o700 {
            return Err(InstallError::Denied(
                "adopted vault must be mode 0700".into(),
            ));
        }
    } else {
        let _ = Command::new("/usr/sbin/chown")
            .arg(format!("{}:{}", req.service_user, req.service_group))
            .arg(&req.vault)
            .status();
        let mut perms = fs::metadata(&req.vault)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&req.vault, perms)?;
    }

    let vault_real = resolve_path(&req.vault);
    if !vault_real.starts_with("/private/var/db/") {
        return Err(InstallError::Denied(
            "vault resolves outside /private/var/db".into(),
        ));
    }

    // Self binary sources.
    let self_exe = std::env::current_exe()
        .map_err(|e| InstallError::Denied(format!("cannot resolve current exe: {e}")))?;
    let client_dst = PathBuf::from(PREFIX).join("bin/sudo-secretspec");
    let broker_dst = PathBuf::from(PREFIX).join("libexec/sudo-secretspec");
    let share = PathBuf::from(PREFIX).join("share/sudo-secretspec");
    let declarations_dst = share.join("secretspec.toml");
    let retired_dst = share.join("sudo-secretspec-retired.toml");
    let guidance_dst = share.join("AI-GUIDANCE.md");
    let manifest_dst = share.join("MANIFEST.sha256");
    let config_dst = PathBuf::from(CONFIG_PATH);
    let sudoers_dst = PathBuf::from(SUDOERS_PATH);

    fs::create_dir_all(PathBuf::from(PREFIX).join("bin"))?;
    fs::create_dir_all(PathBuf::from(PREFIX).join("libexec"))?;
    fs::create_dir_all(PathBuf::from(PREFIX).join("etc"))?;
    fs::create_dir_all(&share)?;

    // Preserve the outgoing artifacts before anything is overwritten, so the
    // snapshot this install reports is actually restorable.
    let stamp = chrono_like_stamp();
    let libexec = PathBuf::from(PREFIX).join("libexec");
    let rollback = libexec.join(format!("{SNAPSHOT_PREFIX}{stamp}"));
    let captured = capture_snapshot(&rollback)?;

    install_file(&self_exe, &client_dst, require_mode(&client_dst)?)?;
    install_file(&self_exe, &broker_dst, require_mode(&broker_dst)?)?;
    install_file(
        &req.declarations,
        &declarations_dst,
        require_mode(&declarations_dst)?,
    )?;
    write_bytes(
        &retired_dst,
        retired_toml().as_bytes(),
        require_mode(&retired_dst)?,
    )?;
    write_bytes(
        &guidance_dst,
        guidance_text().as_bytes(),
        require_mode(&guidance_dst)?,
    )?;
    write_bytes(
        &config_dst,
        config_toml(&req, &vault_real).as_bytes(),
        require_mode(&config_dst)?,
    )?;
    // Validated before it can take effect, and rolled back if the combined
    // configuration is rejected — a broken sudoers.d file would leave the
    // operator unable to elevate at all.
    install_sudoers(
        &sudoers_dst,
        &sudoers_text(&req.operator),
        require_mode(&sudoers_dst)?,
    )?;

    // Release manifest of installed artifacts.
    let mut manifest = String::new();
    for path in [
        &client_dst,
        &broker_dst,
        &declarations_dst,
        &retired_dst,
        &guidance_dst,
        &config_dst,
        &sudoers_dst,
    ] {
        manifest.push_str(&format!("{}  {}\n", sha256_file(path)?, path.display()));
    }
    write_bytes(
        &manifest_dst,
        manifest.as_bytes(),
        require_mode(&manifest_dst)?,
    )?;

    // Runtime files for fresh install only.
    if !req.adopt_existing {
        let manifest_rt = req.vault.join("secretspec.toml");
        let env_rt = req.vault.join(".env");
        fs::copy(&declarations_dst, &manifest_rt)?;
        fs::File::create(&env_rt)?;
        for path in [&manifest_rt, &env_rt] {
            let mut p = fs::metadata(path)?.permissions();
            p.set_mode(0o600);
            fs::set_permissions(path, p)?;
            let _ = Command::new("/usr/sbin/chown")
                .arg(format!("{}:{}", req.service_user, req.service_group))
                .arg(path)
                .status();
        }
    } else {
        for runtime in ["secretspec.toml", ".env"] {
            let p = req.vault.join(runtime);
            if !p.is_file() || p.is_symlink() {
                return Err(InstallError::Denied(format!(
                    "adopted runtime file missing or symlinked: {}",
                    p.display()
                )));
            }
        }
    }

    // Hygiene, after the install itself has succeeded: every install captures
    // a snapshot, and a first install captures an empty one that can never be
    // restored from. Unbounded, they accumulate for the life of the host.
    let pruned = prune_snapshots(&libexec, SNAPSHOT_KEEP);

    println!("installed sudo-secretspec");
    println!("config={}", config_dst.display());
    println!("vault={}", req.vault.display());
    if captured == 0 {
        // Pruned just above; naming it would point at a directory that is gone.
        println!("rollback_snapshot=none");
    } else {
        println!("rollback_snapshot={}", rollback.display());
    }
    println!("rollback_artifacts={captured}");
    println!("pruned_snapshots={pruned}");
    if created_vault {
        println!("created_vault=1");
    }
    Ok(())
}

fn chrono_like_stamp() -> String {
    // UTC-ish timestamp without extra deps.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
