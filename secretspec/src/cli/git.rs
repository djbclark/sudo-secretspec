use super::{load_secrets, shell_quote};
use crate::{GlobalConfig, Secrets};
use clap::Subcommand;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::NamedTempFile;
use url::{Position, Url};

const MARKER_KEY: &str = "secretspec.gitCredentialVersion";
const STATE_KEY: &str = "secretspec.gitCredential";
const FORMAT_VERSION: u8 = 1;

#[derive(Subcommand)]
pub(super) enum GitAction {
    #[command(about = "Configure Git to retrieve an HTTP(S) credential through SecretSpec (0.20+)")]
    Configure {
        #[arg(long, help = "HTTP or HTTPS URL this credential may authenticate")]
        url: Url,
        #[arg(long, help = "SecretSpec key containing the password or token")]
        token_secret: String,
        #[arg(
            long,
            conflicts_with = "username_secret",
            help = "Non-secret username to store in the managed Git configuration"
        )]
        username: Option<String>,
        #[arg(
            long,
            conflicts_with = "username",
            help = "SecretSpec key containing the username"
        )]
        username_secret: Option<String>,
        #[arg(
            short = 'P',
            long,
            env = "SECRETSPEC_PROFILE",
            help = "SecretSpec profile the helper should use"
        )]
        profile: Option<String>,
        #[arg(
            short,
            long,
            env = "SECRETSPEC_PROVIDER",
            help = "Provider override the helper should use"
        )]
        provider: Option<String>,
        #[arg(long, help = "Configure the current user's global Git settings")]
        global: bool,
        #[arg(
            short,
            long,
            requires = "global",
            help = "Confirm a global change non-interactively"
        )]
        yes: bool,
    },
    #[command(about = "Remove Git credential configuration managed by SecretSpec (0.20+)")]
    Unconfigure {
        #[arg(
            long,
            required_unless_present = "all",
            conflicts_with = "all",
            help = "HTTP or HTTPS URL whose managed credential should be removed"
        )]
        url: Option<Url>,
        #[arg(
            long,
            help = "Remove every credential managed by SecretSpec in this scope"
        )]
        all: bool,
        #[arg(long, help = "Operate on the current user's global Git settings")]
        global: bool,
        #[arg(
            short,
            long,
            requires = "global",
            help = "Confirm a global change non-interactively"
        )]
        yes: bool,
    },
}

#[derive(Clone, Copy)]
enum Scope {
    Local,
    Global,
}

impl Scope {
    fn from_global(global: bool) -> Self {
        if global { Self::Global } else { Self::Local }
    }

    fn git_arg(self) -> &'static str {
        match self {
            Self::Local => "--local",
            Self::Global => "--global",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Local => "repository",
            Self::Global => "global",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ManagedCredential {
    version: u8,
    url: String,
    helper: String,
    username: Option<String>,
}

pub(super) fn run(
    action: GitAction,
    file: &Option<PathBuf>,
    reason: &Option<String>,
) -> Result<()> {
    match action {
        GitAction::Configure {
            url,
            token_secret,
            username,
            username_secret,
            profile,
            provider,
            global,
            yes,
        } => configure(ConfigureOptions {
            url,
            token_secret,
            username,
            username_secret,
            profile,
            provider,
            global,
            yes,
            file,
            reason,
        }),
        GitAction::Unconfigure {
            url,
            all,
            global,
            yes,
        } => unconfigure(url, all, Scope::from_global(global), yes),
    }
}

struct ConfigureOptions<'a> {
    url: Url,
    token_secret: String,
    username: Option<String>,
    username_secret: Option<String>,
    profile: Option<String>,
    provider: Option<String>,
    global: bool,
    yes: bool,
    file: &'a Option<PathBuf>,
    reason: &'a Option<String>,
}

fn configure(options: ConfigureOptions<'_>) -> Result<()> {
    crate::integration::git::validate_target(&options.url)?;
    validate_literal_username(options.username.as_deref())?;

    let manifest = manifest_path(options.file)?;
    let mut secrets = load_secrets(options.file, options.reason)?;
    if let Some(profile) = &options.profile {
        secrets.set_profile(profile);
    }
    if let Some(provider) = &options.provider {
        secrets.set_provider(provider);
    }
    let profile = secrets.resolve_profile_name(None);
    validate_secret(&secrets, &options.token_secret, &profile)?;
    if let Some(secret) = &options.username_secret {
        validate_secret(&secrets, secret, &profile)?;
    }

    let target = canonical_target(&options.url);
    let helper = helper_command(
        &target,
        &manifest,
        &profile,
        options.provider.as_deref(),
        options.reason.as_deref(),
        &options.token_secret,
        options.username_secret.as_deref(),
    );
    let credential = ManagedCredential {
        version: FORMAT_VERSION,
        url: target.clone(),
        helper,
        username: options.username,
    };
    let scope = Scope::from_global(options.global);
    let managed_path = managed_path(scope)?;
    let existing = load_managed(&managed_path)?;
    let previous = read_optional(&managed_path)?;
    let mut credentials = existing.clone();
    let changed = match credentials.iter().position(|entry| entry.url == target) {
        Some(index) if credentials[index] == credential => false,
        Some(index) => {
            credentials[index] = credential;
            true
        }
        None => {
            credentials.push(credential);
            true
        }
    };
    let includes = include_paths(scope)?;
    let include_present = includes.iter().any(|path| path == &managed_path);

    if !changed && include_present {
        println!(
            "Git credential for {target} is already configured in {} scope.",
            scope.label()
        );
        return Ok(());
    }

    if !confirm_global(
        scope,
        options.yes,
        &format!("Configure Git credential for {target} in global scope?"),
    )? {
        return Ok(());
    }

    ensure_unchanged(
        scope,
        &managed_path,
        &existing,
        previous.as_deref(),
        &includes,
    )?;
    write_managed(&managed_path, &credentials)?;
    if !include_present && let Err(error) = add_include(scope, &managed_path) {
        restore_managed(&managed_path, previous.as_deref())?;
        return Err(error);
    }

    println!(
        "Configured Git credential for {target} in {} scope.",
        scope.label()
    );
    println!("SecretSpec manifest: {}", manifest.display());
    println!("Undo with: {}", undo_command(scope, &target));
    Ok(())
}

fn unconfigure(url: Option<Url>, all: bool, scope: Scope, yes: bool) -> Result<()> {
    let target = match url {
        Some(url) => {
            crate::integration::git::validate_target(&url)?;
            Some(canonical_target(&url))
        }
        None => None,
    };
    let managed_path = managed_path(scope)?;
    let existing = load_managed(&managed_path)?;
    let previous = read_optional(&managed_path)?;
    let managed_exists = previous.is_some();
    let mut credentials = existing.clone();
    let includes = include_paths(scope)?;
    let include_count = includes
        .iter()
        .filter(|path| *path == &managed_path)
        .count();

    if all {
        if credentials.is_empty() && include_count == 0 && !managed_exists {
            println!(
                "No SecretSpec-managed Git credentials found in {} scope.",
                scope.label()
            );
            return Ok(());
        }
    } else {
        let target = target.as_deref().expect("clap requires --url or --all");
        if !credentials.iter().any(|entry| entry.url == target) {
            println!(
                "No SecretSpec-managed Git credential for {target} was found in {} scope.",
                scope.label()
            );
            return Ok(());
        }
    }

    let description = if all {
        "Remove all SecretSpec-managed Git credential configuration from global scope?".to_string()
    } else {
        format!(
            "Remove the SecretSpec-managed Git credential for {} from global scope?",
            target.as_deref().expect("clap requires --url or --all")
        )
    };
    if !confirm_global(scope, yes, &description)? {
        return Ok(());
    }

    ensure_unchanged(
        scope,
        &managed_path,
        &existing,
        previous.as_deref(),
        &includes,
    )?;
    let removed = if all {
        let removed = credentials.len();
        credentials.clear();
        removed
    } else {
        let before = credentials.len();
        let target = target.as_deref().expect("clap requires --url or --all");
        credentials.retain(|entry| entry.url != target);
        before - credentials.len()
    };

    if credentials.is_empty() {
        if managed_exists {
            write_managed(&managed_path, &[])?;
        }
        remove_includes(scope, &managed_path)?;
        if path_exists(&managed_path)? {
            fs::remove_file(&managed_path)
                .into_diagnostic()
                .wrap_err_with(|| format!("Failed to remove {}", managed_path.display()))?;
        }
    } else {
        write_managed(&managed_path, &credentials)?;
    }

    if all {
        println!(
            "Removed all {removed} SecretSpec-managed Git credential {} from {} scope.",
            if removed == 1 { "entry" } else { "entries" },
            scope.label()
        );
    } else {
        println!(
            "Removed the SecretSpec-managed Git credential for {} from {} scope.",
            target.expect("clap requires --url or --all"),
            scope.label()
        );
    }
    Ok(())
}

fn validate_literal_username(username: Option<&str>) -> Result<()> {
    if let Some(username) = username {
        if username.is_empty() {
            return Err(miette!("Git username cannot be empty"));
        }
        if username.contains(['\n', '\r', '\0']) {
            return Err(miette!("Git username cannot contain a newline or NUL byte"));
        }
    }
    Ok(())
}

fn validate_secret(secrets: &Secrets, name: &str, profile: &str) -> Result<()> {
    if name.is_empty() {
        return Err(miette!("Secret name cannot be empty"));
    }
    let secret = secrets.resolve_secret_config(name, None).ok_or_else(|| {
        miette!("Secret '{name}' is not declared in SecretSpec profile '{profile}'")
    })?;
    if secret.as_path == Some(true) {
        return Err(miette!(
            "Secret '{name}' uses as_path and cannot be returned as a Git credential"
        ));
    }
    Ok(())
}

fn manifest_path(file: &Option<PathBuf>) -> Result<PathBuf> {
    let path = match file {
        Some(path) => path.clone(),
        None => crate::secrets::find_config_file().into_diagnostic()?,
    };
    let path = dunce::canonicalize(&path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to resolve SecretSpec manifest {}", path.display()))?;
    require_utf8_path(path, "SecretSpec manifest")
}

fn canonical_target(url: &Url) -> String {
    let mut target = url[..Position::BeforePath].to_string();
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        target.push_str(path);
    }
    target
}

fn helper_command(
    target: &str,
    manifest: &Path,
    profile: &str,
    provider: Option<&str>,
    reason: Option<&str>,
    token_secret: &str,
    username_secret: Option<&str>,
) -> String {
    let mut command = format!(
        "secretspec --url {} --file {} --profile {} --password-secret {}",
        shell_quote(target),
        shell_quote(&manifest.to_string_lossy()),
        shell_quote(profile),
        shell_quote(token_secret)
    );
    if let Some(secret) = username_secret {
        command.push_str(" --username-secret ");
        command.push_str(&shell_quote(secret));
    }
    if let Some(provider) = provider {
        command.push_str(" --provider ");
        command.push_str(&shell_quote(provider));
    }
    if let Some(reason) = reason {
        command.push_str(" --reason ");
        command.push_str(&shell_quote(reason));
    }
    command
}

fn managed_path(scope: Scope) -> Result<PathBuf> {
    let path = match scope {
        Scope::Local => {
            let output = git_output(["rev-parse", "--git-common-dir"])?;
            let path = output_text(output, "Failed to locate the current Git repository")?;
            let path = path.strip_suffix('\n').unwrap_or(&path);
            let path = path.strip_suffix('\r').unwrap_or(path);
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().into_diagnostic()?.join(path)
            };
            dunce::canonicalize(&path)
                .into_diagnostic()
                .wrap_err_with(|| format!("Failed to resolve Git directory {}", path.display()))?
                .join("secretspec-credentials")
        }
        Scope::Global => {
            let config = GlobalConfig::path().into_diagnostic()?;
            let directory = config
                .parent()
                .ok_or_else(|| miette!("SecretSpec config path has no parent directory"))?;
            directory.join("git-credentials")
        }
    };
    require_utf8_path(path, "managed Git configuration")
}

fn require_utf8_path(path: PathBuf, description: &str) -> Result<PathBuf> {
    if path.to_str().is_none() {
        return Err(miette!(
            "{description} path must be valid UTF-8 for Git helper configuration"
        ));
    }
    Ok(path)
}

fn include_paths(scope: Scope) -> Result<Vec<PathBuf>> {
    let output = git_output([
        "config",
        scope.git_arg(),
        "--no-includes",
        "--null",
        "--get-all",
        "include.path",
    ])?;
    if output.status.success() {
        Ok(split_nul(&output.stdout)
            .into_iter()
            .map(PathBuf::from)
            .collect())
    } else if output.status.code() == Some(1) {
        Ok(Vec::new())
    } else {
        Err(command_error("Failed to read Git include paths", &output))
    }
}

fn ensure_unchanged(
    scope: Scope,
    path: &Path,
    credentials: &[ManagedCredential],
    contents: Option<&[u8]>,
    includes: &[PathBuf],
) -> Result<()> {
    if load_managed(path)? != credentials
        || read_optional(path)?.as_deref() != contents
        || include_paths(scope)? != includes
    {
        return Err(miette!(
            "Git credential configuration changed during this operation; no changes were made; rerun the command"
        ));
    }
    Ok(())
}

fn add_include(scope: Scope, path: &Path) -> Result<()> {
    run_git(
        [
            "config".into(),
            scope.git_arg().into(),
            "--add".into(),
            "include.path".into(),
            path.as_os_str().into(),
        ],
        "Failed to register the SecretSpec Git configuration",
    )
}

fn remove_includes(scope: Scope, path: &Path) -> Result<()> {
    loop {
        let includes = include_paths(scope)?;
        if !includes.iter().any(|include| include == path) {
            return Ok(());
        }
        let path = path.to_string_lossy().into_owned();
        run_git(
            [
                "config".into(),
                scope.git_arg().into(),
                "--no-includes".into(),
                "--unset".into(),
                "--fixed-value".into(),
                "include.path".into(),
                path.into(),
            ],
            "Failed to unregister the SecretSpec Git configuration",
        )?;
    }
}

fn load_managed(path: &Path) -> Result<Vec<ManagedCredential>> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let markers = config_values(path, MARKER_KEY)?;
    if markers != [FORMAT_VERSION.to_string()] {
        return Err(unmanaged_file_error(path));
    }
    let raw_states = config_values(path, STATE_KEY)?;
    let mut credentials = Vec::with_capacity(raw_states.len());
    let mut urls = HashSet::new();
    for raw in raw_states {
        let credential: ManagedCredential =
            serde_json::from_str(&raw).map_err(|_| unmanaged_file_error(path))?;
        let parsed = Url::parse(&credential.url).map_err(|_| unmanaged_file_error(path))?;
        if credential.version != FORMAT_VERSION
            || crate::integration::git::validate_target(&parsed).is_err()
            || canonical_target(&parsed) != credential.url
            || !urls.insert(credential.url.clone())
        {
            return Err(unmanaged_file_error(path));
        }
        credentials.push(credential);
    }
    verify_managed_file(path, &credentials)?;
    Ok(credentials)
}

fn verify_managed_file(path: &Path, credentials: &[ManagedCredential]) -> Result<()> {
    let mut expected_names = vec![MARKER_KEY.to_ascii_lowercase()];
    for credential in credentials {
        expected_names.push(helper_key(&credential.url).to_ascii_lowercase());
        if credential.username.is_some() {
            expected_names.push(username_key(&credential.url).to_ascii_lowercase());
        }
        if target_has_path(&credential.url) {
            expected_names.push(use_http_path_key(&credential.url).to_ascii_lowercase());
        }
        expected_names.push(STATE_KEY.to_ascii_lowercase());
    }
    expected_names.sort();

    let output = git_output_os([
        "config".into(),
        "--file".into(),
        path.as_os_str().into(),
        "--no-includes".into(),
        "--null".into(),
        "--name-only".into(),
        "--get-regexp".into(),
        ".*".into(),
    ])?;
    if !output.status.success() {
        return Err(unmanaged_file_error(path));
    }
    let mut actual_names: Vec<_> = split_nul(&output.stdout)
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    actual_names.sort();
    if actual_names != expected_names {
        return Err(unmanaged_file_error(path));
    }

    for credential in credentials {
        if config_values(path, &helper_key(&credential.url))? != [credential.helper.clone()] {
            return Err(unmanaged_file_error(path));
        }
        let expected_username: Vec<_> = credential.username.clone().into_iter().collect();
        if config_values(path, &username_key(&credential.url))? != expected_username {
            return Err(unmanaged_file_error(path));
        }
        let expected_use_http_path = if target_has_path(&credential.url) {
            vec!["true".to_string()]
        } else {
            Vec::new()
        };
        if config_values(path, &use_http_path_key(&credential.url))? != expected_use_http_path {
            return Err(unmanaged_file_error(path));
        }
    }
    Ok(())
}

fn write_managed(path: &Path, credentials: &[ManagedCredential]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| miette!("Managed Git configuration path has no parent directory"))?;
    fs::create_dir_all(directory)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to create {}", directory.display()))?;
    let mut temporary = NamedTempFile::new_in(directory)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to create temporary file in {}", directory.display()))?;
    config_add(temporary.path(), MARKER_KEY, &FORMAT_VERSION.to_string())?;
    let mut credentials = credentials.to_vec();
    credentials.sort_by(|left, right| left.url.cmp(&right.url));
    for credential in credentials {
        config_add(
            temporary.path(),
            &helper_key(&credential.url),
            &credential.helper,
        )?;
        if let Some(username) = &credential.username {
            config_add(temporary.path(), &username_key(&credential.url), username)?;
        }
        if target_has_path(&credential.url) {
            config_add(
                temporary.path(),
                &use_http_path_key(&credential.url),
                "true",
            )?;
        }
        let state = serde_json::to_string(&credential).into_diagnostic()?;
        config_add(temporary.path(), STATE_KEY, &state)?;
    }
    temporary.flush().into_diagnostic()?;
    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .into_diagnostic()?;
    temporary.as_file().sync_all().into_diagnostic()?;
    temporary.persist(path).map_err(|error| {
        miette!(
            "Failed to atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

fn restore_managed(path: &Path, previous: Option<&[u8]>) -> Result<()> {
    match previous {
        Some(contents) => {
            let directory = path
                .parent()
                .ok_or_else(|| miette!("Managed Git configuration path has no parent directory"))?;
            let mut temporary = NamedTempFile::new_in(directory).into_diagnostic()?;
            temporary.write_all(contents).into_diagnostic()?;
            temporary.flush().into_diagnostic()?;
            temporary.persist(path).map_err(|error| {
                miette!("Failed to restore {}: {}", path.display(), error.error)
            })?;
        }
        None => {
            if path_exists(path)? {
                fs::remove_file(path).into_diagnostic()?;
            }
        }
    }
    Ok(())
}

fn config_add(path: &Path, key: &str, value: &str) -> Result<()> {
    run_git(
        [
            "config".into(),
            "--file".into(),
            path.as_os_str().into(),
            "--add".into(),
            key.into(),
            value.into(),
        ],
        "Failed to write managed Git configuration",
    )
}

fn path_exists(path: &Path) -> Result<bool> {
    path.try_exists()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to inspect {}", path.display()))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to read {}", path.display())),
    }
}

fn config_values(path: &Path, key: &str) -> Result<Vec<String>> {
    let output = git_output_os([
        "config".into(),
        "--file".into(),
        path.as_os_str().into(),
        "--no-includes".into(),
        "--null".into(),
        "--get-all".into(),
        key.into(),
    ])?;
    if output.status.success() {
        Ok(split_nul(&output.stdout))
    } else if output.status.code() == Some(1) {
        Ok(Vec::new())
    } else {
        Err(command_error(
            "Failed to read managed Git configuration",
            &output,
        ))
    }
}

fn helper_key(url: &str) -> String {
    format!("credential.{url}.helper")
}

fn username_key(url: &str) -> String {
    format!("credential.{url}.username")
}

fn use_http_path_key(url: &str) -> String {
    format!("credential.{url}.useHttpPath")
}

fn target_has_path(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| !url.path().trim_matches('/').is_empty())
}

fn confirm_global(scope: Scope, yes: bool, prompt: &str) -> Result<bool> {
    if !matches!(scope, Scope::Global) || yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(miette!(
            "refusing to change global Git configuration without confirmation; pass --yes for non-interactive use"
        ));
    }
    let confirmed = inquire::Confirm::new(prompt)
        .with_default(false)
        .prompt()
        .into_diagnostic()?;
    if confirmed {
        Ok(true)
    } else {
        println!("Cancelled.");
        Ok(false)
    }
}

fn undo_command(scope: Scope, target: &str) -> String {
    format!(
        "secretspec git unconfigure --url {}{}",
        shell_quote(target),
        if matches!(scope, Scope::Global) {
            " --global"
        } else {
            ""
        }
    )
}

fn unmanaged_file_error(path: &Path) -> miette::Report {
    miette!(
        "{} is not an intact SecretSpec-managed Git configuration; refusing to modify it; inspect the file and its Git include before removing either manually",
        path.display()
    )
}

fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect()
}

fn git_output<const N: usize>(args: [&str; N]) -> Result<Output> {
    git_output_os(args.map(Into::into))
}

fn git_output_os<const N: usize>(args: [std::ffi::OsString; N]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .output()
        .into_diagnostic()
        .wrap_err("Failed to run Git")
}

fn run_git<const N: usize>(args: [std::ffi::OsString; N], context: &str) -> Result<()> {
    let output = git_output_os(args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(context, &output))
    }
}

fn output_text(output: Output, context: &str) -> Result<String> {
    if !output.status.success() {
        return Err(command_error(context, &output));
    }
    String::from_utf8(output.stdout)
        .into_diagnostic()
        .wrap_err(context.to_string())
}

fn command_error(context: &str, output: &Output) -> miette::Report {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        miette!("{context}")
    } else {
        miette!("{context}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn credential(url: &str, username: Option<&str>) -> ManagedCredential {
        ManagedCredential {
            version: FORMAT_VERSION,
            url: url.to_string(),
            helper: format!("secretspec --url '{url}' --password-secret 'TOKEN'"),
            username: username.map(str::to_string),
        }
    }

    #[test]
    fn canonical_target_normalizes_root_and_trailing_slashes() {
        assert_eq!(
            canonical_target(&Url::parse("https://GITHUB.com/").unwrap()),
            "https://github.com"
        );
        assert_eq!(
            canonical_target(&Url::parse("https://github.com/cachix///").unwrap()),
            "https://github.com/cachix"
        );
    }

    #[test]
    fn managed_file_round_trips_exact_owned_entries() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("credentials");
        let credentials = vec![
            credential("https://gitlab.com", None),
            credential("https://github.com", Some("vimjoyer")),
        ];

        write_managed(&path, &credentials).unwrap();
        let mut loaded = load_managed(&path).unwrap();
        loaded.sort_by(|left, right| left.url.cmp(&right.url));
        let mut expected = credentials;
        expected.sort_by(|left, right| left.url.cmp(&right.url));
        assert_eq!(loaded, expected);
    }

    #[test]
    fn path_scoped_credential_enables_http_paths() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("credentials");
        let target = "https://github.com/cachix";
        write_managed(&path, &[credential(target, None)]).unwrap();
        assert_eq!(
            config_values(&path, &use_http_path_key(target)).unwrap(),
            ["true"]
        );
        assert_eq!(load_managed(&path).unwrap(), [credential(target, None)]);
    }

    #[test]
    fn empty_managed_file_remains_recognizable() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("credentials");
        write_managed(&path, &[]).unwrap();
        assert!(load_managed(&path).unwrap().is_empty());
    }

    #[test]
    fn refuses_to_modify_managed_file_with_unknown_entries() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("credentials");
        write_managed(&path, &[credential("https://github.com", None)]).unwrap();
        config_add(&path, "user.name", "do not delete").unwrap();
        let error = load_managed(&path).unwrap_err();
        assert!(error.to_string().contains("refusing to modify"));
        assert_eq!(
            config_values(&path, "user.name").unwrap(),
            ["do not delete"]
        );
    }

    #[test]
    fn helper_command_quotes_every_persisted_argument() {
        let command = helper_command(
            "https://github.com",
            Path::new("/tmp/project's secretspec.toml"),
            "team's profile",
            Some("provider's alias"),
            Some("developer's machine"),
            "TOKEN'S_NAME",
            Some("USERNAME'S_NAME"),
        );
        assert!(command.contains("'/tmp/project'\\''s secretspec.toml'"));
        assert!(command.contains("'TOKEN'\\''S_NAME'"));
        assert!(command.contains("'USERNAME'\\''S_NAME'"));
        assert!(command.contains("'provider'\\''s alias'"));
        assert!(command.contains("'developer'\\''s machine'"));
    }
}
