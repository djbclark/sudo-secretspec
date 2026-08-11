use crate::{NamedResolution, Secrets};
use clap::Parser;
use miette::{IntoDiagnostic, Result, miette};
use secrecy::{ExposeSecret, SecretString};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use url::{Host, Url};

const MAX_ATTRIBUTE_LINE_BYTES: usize = 65_535;

#[derive(Parser)]
#[command(
    name = "git-credential-secretspec",
    about = "Retrieve Git HTTP(S) credentials through SecretSpec providers",
    version
)]
struct Args {
    #[arg(long, help = "Git URL this credential is allowed to authenticate")]
    url: Url,
    #[arg(long, help = "SecretSpec key containing the Git username")]
    username_secret: Option<String>,
    #[arg(long, help = "SecretSpec key containing the Git password or token")]
    password_secret: String,
    #[arg(
        short = 'f',
        long,
        env = "SECRETSPEC_FILE",
        help = "Path to secretspec.toml"
    )]
    file: Option<PathBuf>,
    #[arg(
        short = 'P',
        long,
        env = "SECRETSPEC_PROFILE",
        help = "SecretSpec profile to use"
    )]
    profile: Option<String>,
    #[arg(
        short,
        long,
        env = "SECRETSPEC_PROVIDER",
        help = "Override the SecretSpec provider"
    )]
    provider: Option<String>,
    #[arg(
        long,
        env = "SECRETSPEC_REASON",
        help = "Reason recorded for the secret access"
    )]
    reason: Option<String>,
    #[arg(help = "Git credential operation")]
    operation: String,
}

#[derive(Default)]
struct Request {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

impl Request {
    fn read(mut input: impl BufRead) -> Result<Self> {
        let mut request = Self::default();
        let mut line = String::new();

        loop {
            line.clear();
            let read = input.read_line(&mut line).into_diagnostic()?;
            if read == 0 {
                break;
            }
            if read > MAX_ATTRIBUTE_LINE_BYTES {
                return Err(miette!("Git credential attribute exceeds 65535 bytes"));
            }
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            if line.is_empty() {
                break;
            }
            if line.contains('\0') {
                return Err(miette!("invalid Git credential attribute"));
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| miette!("invalid Git credential attribute"))?;
            match key {
                "protocol" => request.protocol = Some(value.to_string()),
                "host" => request.host = Some(value.to_string()),
                "path" => request.path = Some(value.to_string()),
                "url" => request.apply_url(value)?,
                _ => {}
            }
        }

        Ok(request)
    }

    fn apply_url(&mut self, value: &str) -> Result<()> {
        let parsed = Url::parse(value).into_diagnostic()?;
        self.protocol = Some(parsed.scheme().to_string());
        if let Some(host) = parsed.host() {
            let host = match host {
                Host::Ipv6(address) => format!("[{address}]"),
                host => host.to_string(),
            };
            self.host = Some(match parsed.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            });
        }
        let path = parsed.path().trim_start_matches('/');
        if !path.is_empty() {
            self.path = Some(path.to_string());
        }
        Ok(())
    }

    fn authority_url(&self) -> Option<Url> {
        let protocol = self.protocol.as_deref()?;
        let host = self.host.as_deref()?;
        let candidate = Url::parse(&format!("{protocol}://{host}/")).ok()?;
        (candidate.host().is_some()
            && candidate.username().is_empty()
            && candidate.password().is_none()
            && candidate.path() == "/"
            && candidate.query().is_none()
            && candidate.fragment().is_none())
        .then_some(candidate)
    }
}

pub(crate) fn validate_target(target: &Url) -> Result<()> {
    if !matches!(target.scheme(), "http" | "https") {
        return Err(miette!("Git credential URL must use HTTP or HTTPS"));
    }
    if target.host().is_none() {
        return Err(miette!("Git credential URL must include a host"));
    }
    if !target.username().is_empty() || target.password().is_some() {
        return Err(miette!("Git credential URL must not include credentials"));
    }
    if target.query().is_some() || target.fragment().is_some() {
        return Err(miette!(
            "Git credential URL must not include a query or fragment"
        ));
    }
    Ok(())
}

fn target_matches(target: &Url, request: &Request) -> bool {
    let Some(candidate) = request.authority_url() else {
        return false;
    };
    if target.scheme() != candidate.scheme()
        || target.host() != candidate.host()
        || target.port_or_known_default() != candidate.port_or_known_default()
    {
        return false;
    }

    let expected = target.path().trim_matches('/');
    if expected.is_empty() {
        return true;
    }
    let actual = request
        .path
        .as_deref()
        .unwrap_or_default()
        .trim_matches('/');
    actual == expected
        || actual
            .strip_prefix(expected)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn validate_value(name: &str, attribute: &str, value: &SecretString) -> Result<()> {
    let value = value.expose_secret();
    if value.contains(['\n', '\r', '\0']) {
        return Err(miette!(
            "Secret '{name}' cannot be represented by Git's credential protocol"
        ));
    }
    if attribute.len() + value.len() + 2 > MAX_ATTRIBUTE_LINE_BYTES {
        return Err(miette!(
            "Secret '{name}' exceeds Git's credential protocol line limit"
        ));
    }
    Ok(())
}

fn load(args: &Args) -> Result<Secrets> {
    let mut secrets = match &args.file {
        Some(path) => Secrets::load_from(path),
        None => Secrets::load(),
    }?;
    if let Some(provider) = &args.provider {
        secrets.set_provider(provider);
    }
    if let Some(profile) = &args.profile {
        secrets.set_profile(profile);
    }
    if let Some(reason) = &args.reason {
        secrets = secrets.with_reason(reason);
    }
    secrets.set_ignore_ambient_scope(true);
    Ok(secrets)
}

fn resolve(secrets: &Secrets, name: &str) -> Result<Option<SecretString>> {
    let Some(config) = secrets.resolve_secret_config(name, None) else {
        return Err(miette!(
            "Secret '{name}' is not declared in the selected SecretSpec profile"
        ));
    };
    if config.as_path == Some(true) {
        return Err(miette!(
            "Secret '{name}' uses as_path and cannot be returned as a Git credential"
        ));
    }
    match secrets.resolve_named(name)? {
        NamedResolution::Resolved(secret) => {
            let value = secret.value.ok_or_else(|| {
                miette!("Secret '{name}' uses as_path and cannot be returned as a Git credential")
            })?;
            Ok(Some(SecretString::new(value.into())))
        }
        NamedResolution::Missing { .. } => Ok(None),
        NamedResolution::Undeclared => Err(miette!(
            "Secret '{name}' is not declared in the selected SecretSpec profile"
        )),
    }
}

fn run(args: Args, input: impl BufRead, mut output: impl Write) -> Result<()> {
    if args.operation != "get" {
        return Ok(());
    }

    validate_target(&args.url)?;
    let request = Request::read(input)?;

    if !target_matches(&args.url, &request) {
        return Ok(());
    }

    let secrets = load(&args)?;
    let Some(password) = resolve(&secrets, &args.password_secret)? else {
        return Ok(());
    };
    let username = match &args.username_secret {
        Some(name) => {
            let Some(value) = resolve(&secrets, name)? else {
                return Ok(());
            };
            Some((name, value))
        }
        None => None,
    };

    validate_value(&args.password_secret, "password", &password)?;
    if let Some((name, value)) = &username {
        validate_value(name, "username", value)?;
    }

    if let Some((_, username)) = username {
        writeln!(output, "username={}", username.expose_secret()).into_diagnostic()?;
    }
    writeln!(output, "password={}", password.expose_secret()).into_diagnostic()?;
    writeln!(output).into_diagnostic()?;
    Ok(())
}

pub fn main() -> Result<()> {
    run(
        Args::parse(),
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use tempfile::TempDir;

    fn manifest(contents: &str) -> (TempDir, PathBuf) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("secretspec.toml");
        fs::write(&path, contents).unwrap();
        (directory, path)
    }

    fn args(path: PathBuf, operation: &str) -> Args {
        Args {
            url: Url::parse("https://github.com").unwrap(),
            username_secret: Some("GITHUB_USERNAME".to_string()),
            password_secret: "GITHUB_TOKEN".to_string(),
            file: Some(path),
            profile: None,
            provider: None,
            reason: None,
            operation: operation.to_string(),
        }
    }

    #[test]
    fn returns_declared_credentials_for_matching_url() {
        let (_directory, path) = manifest(
            r#"
[project]
name = "git-helper"
revision = "1.0"
require_reason = false

[profiles.default]
GITHUB_USERNAME = { description = "GitHub username", default = "vimjoyer", providers = ["null"] }
GITHUB_TOKEN = { description = "GitHub token", default = "token=value", providers = ["null"] }
"#,
        );
        let mut output = Vec::new();
        run(
            args(path, "get"),
            Cursor::new("protocol=https\nhost=github.com\n\n"),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "username=vimjoyer\npassword=token=value\n\n"
        );
    }

    #[test]
    fn mismatched_url_does_not_load_or_return_credentials() {
        let mut output = Vec::new();
        run(
            args(PathBuf::from("missing.toml"), "get"),
            Cursor::new("protocol=https\nhost=example.com\n\n"),
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn missing_stored_password_returns_nothing() {
        let (_directory, path) = manifest(
            r#"
[project]
name = "git-helper"
revision = "1.0"
require_reason = false

[profiles.default]
GITHUB_USERNAME = { description = "GitHub username", default = "vimjoyer", providers = ["null"] }
GITHUB_TOKEN = { description = "GitHub token", providers = ["null"] }
"#,
        );
        let mut output = Vec::new();
        run(
            args(path, "get"),
            Cursor::new("protocol=https\nhost=github.com\n\n"),
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn unsupported_operations_do_not_read_or_load() {
        for operation in ["store", "erase", "capability", "future-operation"] {
            let mut arguments = args(PathBuf::from("missing.toml"), operation);
            arguments.url = Url::parse("ssh://github.com").unwrap();
            let mut output = Vec::new();
            run(
                arguments,
                Cursor::new("not a credential attribute\n"),
                &mut output,
            )
            .unwrap();
            assert!(output.is_empty());
        }
    }

    #[test]
    fn configured_path_matches_only_its_path_segment() {
        let target = Url::parse("https://github.com/cachix").unwrap();
        let matching = Request {
            protocol: Some("https".to_string()),
            host: Some("github.com".to_string()),
            path: Some("cachix/secretspec".to_string()),
        };
        let unrelated = Request {
            protocol: Some("https".to_string()),
            host: Some("github.com".to_string()),
            path: Some("cachix-evil/secretspec".to_string()),
        };
        assert!(target_matches(&target, &matching));
        assert!(!target_matches(&target, &unrelated));
    }

    #[test]
    fn different_protocol_or_similar_hostname_does_not_match() {
        let target = Url::parse("https://github.com").unwrap();
        for request in [
            Request {
                protocol: Some("http".to_string()),
                host: Some("github.com".to_string()),
                path: None,
            },
            Request {
                protocol: Some("https".to_string()),
                host: Some("github.com.example.com".to_string()),
                path: None,
            },
            Request {
                protocol: Some("https".to_string()),
                host: Some("example.com@github.com".to_string()),
                path: None,
            },
            Request {
                protocol: Some("https".to_string()),
                host: Some("github.com/path".to_string()),
                path: None,
            },
        ] {
            assert!(!target_matches(&target, &request));
        }
    }

    #[test]
    fn url_attribute_and_default_port_match() {
        let mut request = Request::default();
        request
            .apply_url("https://github.com:443/cachix/secretspec")
            .unwrap();
        assert!(target_matches(
            &Url::parse("https://github.com/cachix").unwrap(),
            &request
        ));
    }

    #[test]
    fn rejects_values_that_git_cannot_represent() {
        for value in ["line\nfeed", "carriage\rreturn", "nul\0byte"] {
            let value = SecretString::new(value.to_string().into());
            assert!(validate_value("TOKEN", "password", &value).is_err());
        }

        let value = SecretString::new("x".repeat(MAX_ATTRIBUTE_LINE_BYTES).into());
        assert!(validate_value("TOKEN", "password", &value).is_err());
    }

    #[test]
    fn rejects_as_path_credentials() {
        let (_directory, path) = manifest(
            r#"
[project]
name = "git-helper"
revision = "1.0"
require_reason = false

[profiles.default]
GITHUB_USERNAME = { description = "GitHub username", default = "vimjoyer", providers = ["null"] }
GITHUB_TOKEN = { description = "GitHub token", default = "token", providers = ["null"], as_path = true }
"#,
        );
        let error = run(
            args(path, "get"),
            Cursor::new("protocol=https\nhost=github.com\n\n"),
            Vec::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("uses as_path"));
    }
}
