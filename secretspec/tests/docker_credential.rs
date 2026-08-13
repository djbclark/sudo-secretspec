use serde_json::Value;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    manifest: PathBuf,
    docker_config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        let manifest = root.join("secretspec.toml");
        fs::write(
            &manifest,
            r#"
[project]
name = "docker-helper"
revision = "1.0"
require_reason = false

[profiles.default]
DOCKER_USERNAME = { description = "Docker username", default = "registry-user", providers = ["null"] }
DOCKER_TOKEN = { description = "Docker token", default = "token=value", providers = ["null"] }
"#,
        )
        .unwrap();
        let docker_directory = root.join("docker");
        fs::create_dir(&docker_directory).unwrap();
        let docker_config = docker_directory.join("config.json");
        fs::write(
            &docker_config,
            r#"{
  "auths": {"example.com": {"auth": "encoded"}},
  "credsStore": "desktop",
  "credHelpers": {"existing.example.com": "pass"},
  "plugins": {"debug": {"hooks": "exec"}}
}
"#,
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            manifest,
            docker_config,
        }
    }

    fn apply_environment(&self, command: &mut Command) {
        command
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("APPDATA", self.root.join("config"))
            .env("LOCALAPPDATA", self.root.join("state"))
            .env("DOCKER_CONFIG", self.root.join("docker"))
            .env_remove("SECRETSPEC_FILE")
            .env_remove("SECRETSPEC_PROFILE")
            .env_remove("SECRETSPEC_PROVIDER")
            .env_remove("SECRETSPEC_REASON");
    }

    fn secretspec(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_secretspec"));
        command.arg("--file").arg(&self.manifest);
        self.apply_environment(&mut command);
        command
    }

    fn embedded_secretspec(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_secretspec"));
        self.apply_environment(&mut command);
        command
    }

    fn helper(&self, operation: &str, input: &[u8]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_docker-credential-secretspec"));
        command
            .arg(operation)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.apply_environment(&mut command);
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    fn configure(&self, registry: &str) -> Output {
        self.secretspec()
            .args([
                "docker",
                "configure",
                "--registry",
                registry,
                "--token-secret",
                "DOCKER_TOKEN",
                "--username-secret",
                "DOCKER_USERNAME",
                "--provider",
                "null",
                "--yes",
            ])
            .output()
            .unwrap()
    }
}

fn command_with_stdin(mut command: Command, args: &[&str], input: &[u8]) -> Output {
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_success(context: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{context} failed with {}:\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn configure_get_and_unconfigure_preserve_docker_configuration() {
    let fixture = Fixture::new();
    let original = read_json(&fixture.docker_config);

    let output = fixture.configure("ghcr.io");
    assert_success("docker configure", &output);
    let configured = read_json(&fixture.docker_config);
    assert_eq!(configured["credHelpers"]["ghcr.io"], "secretspec");
    assert_eq!(configured["credHelpers"]["existing.example.com"], "pass");
    assert_eq!(configured["credsStore"], original["credsStore"]);
    assert_eq!(configured["auths"], original["auths"]);
    assert_eq!(configured["plugins"], original["plugins"]);

    let output = fixture.helper("get", b"ghcr.io\n");
    assert_success("docker credential get", &output);
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["Username"], "registry-user");
    assert_eq!(response["Secret"], "token=value");

    let output = fixture
        .secretspec()
        .args(["docker", "unconfigure", "--registry", "ghcr.io", "--yes"])
        .output()
        .unwrap();
    assert_success("docker unconfigure", &output);
    assert_eq!(read_json(&fixture.docker_config), original);

    let output = fixture.helper("get", b"ghcr.io\n");
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "credentials not found in native keychain"
    );
}

#[test]
fn embedded_credentials_ignore_the_cwd_manifest_and_isolate_each_registry() {
    let fixture = Fixture::new();
    let store = fixture.root.join("docker-credential-store");
    let provider = format!("file://{}", store.display());

    let output = fixture
        .embedded_secretspec()
        .args([
            "docker",
            "configure",
            "--registry",
            "ghcr.io",
            "--username",
            "github-user",
            "--provider",
            &provider,
            "--yes",
        ])
        .output()
        .unwrap();
    assert_success("embedded GHCR configure", &output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("secretspec docker login 'ghcr.io'"));
    assert!(!stdout.contains("SecretSpec manifest:"));

    let output = command_with_stdin(
        fixture.embedded_secretspec(),
        &["docker", "login", "ghcr.io", "--provider", &provider],
        b"github-token\n",
    );
    assert_success("embedded GHCR login", &output);

    let output = fixture
        .embedded_secretspec()
        .args([
            "docker",
            "configure",
            "--registry",
            "registry.example.com:5000",
            "--username",
            "private-user",
            "--provider",
            &provider,
            "--yes",
        ])
        .output()
        .unwrap();
    assert_success("embedded private registry configure", &output);
    let output = command_with_stdin(
        fixture.embedded_secretspec(),
        &[
            "docker",
            "login",
            "registry.example.com:5000",
            "--provider",
            &provider,
        ],
        b"private-token\n",
    );
    assert_success("embedded private registry login", &output);

    let github = fixture.helper("get", b"ghcr.io\n");
    assert_success("embedded GHCR get", &github);
    let github: Value = serde_json::from_slice(&github.stdout).unwrap();
    assert_eq!(github["Username"], "github-user");
    assert_eq!(github["Secret"], "github-token");

    let private = fixture.helper("get", b"registry.example.com:5000\n");
    assert_success("embedded private registry get", &private);
    let private: Value = serde_json::from_slice(&private.stdout).unwrap();
    assert_eq!(private["Username"], "private-user");
    assert_eq!(private["Secret"], "private-token");

    let output = fixture
        .embedded_secretspec()
        .args(["docker", "logout", "ghcr.io", "--provider", &provider])
        .output()
        .unwrap();
    assert_success("embedded GHCR logout", &output);
    assert!(!fixture.helper("get", b"ghcr.io\n").status.success());
    let private = fixture.helper("get", b"registry.example.com:5000\n");
    assert_success("private registry remains after GHCR logout", &private);
}

#[test]
fn embedded_and_custom_manifest_options_cannot_be_mixed() {
    let fixture = Fixture::new();
    let output = fixture
        .embedded_secretspec()
        .args([
            "docker",
            "configure",
            "--registry",
            "ghcr.io",
            "--username",
            "registry-user",
            "--token-secret",
            "DOCKER_TOKEN",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--token-secret") && stderr.contains("require --file"));

    let output = fixture
        .secretspec()
        .args(["docker", "login", "ghcr.io"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("manages the embedded Docker credential store")
    );
}

#[test]
fn repeated_configuration_is_idempotent() {
    let fixture = Fixture::new();
    assert_success("first docker configure", &fixture.configure("ghcr.io"));
    let configured = fs::read(&fixture.docker_config).unwrap();

    let output = fixture.configure("ghcr.io");
    assert_success("second docker configure", &output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("already configured"));
    assert_eq!(fs::read(&fixture.docker_config).unwrap(), configured);
}

#[test]
fn unconfigure_all_removes_only_managed_registry_helpers() {
    let fixture = Fixture::new();
    let original = read_json(&fixture.docker_config);
    assert_success("first docker configure", &fixture.configure("ghcr.io"));
    assert_success(
        "second docker configure",
        &fixture.configure("registry.example.com:5000"),
    );

    let output = fixture
        .secretspec()
        .args(["docker", "unconfigure", "--all", "--yes"])
        .output()
        .unwrap();
    assert_success("docker unconfigure --all", &output);
    assert_eq!(read_json(&fixture.docker_config), original);
}

#[test]
fn configure_refuses_to_replace_an_existing_registry_helper() {
    let fixture = Fixture::new();
    let mut config = read_json(&fixture.docker_config);
    config["credHelpers"]["ghcr.io"] = Value::String("pass".to_string());
    fs::write(
        &fixture.docker_config,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    let original = fs::read(&fixture.docker_config).unwrap();

    let output = fixture.configure("ghcr.io");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already uses credential helper 'pass'"));
    assert!(stderr.contains("refusing"));
    assert_eq!(fs::read(&fixture.docker_config).unwrap(), original);
}

#[test]
fn unconfigure_refuses_to_remove_an_externally_changed_helper() {
    let fixture = Fixture::new();
    assert_success("docker configure", &fixture.configure("ghcr.io"));
    let mut config = read_json(&fixture.docker_config);
    config["credHelpers"]["ghcr.io"] = Value::String("pass".to_string());
    fs::write(
        &fixture.docker_config,
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    let changed = fs::read(&fixture.docker_config).unwrap();

    let output = fixture
        .secretspec()
        .args(["docker", "unconfigure", "--registry", "ghcr.io", "--yes"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed to 'pass'"));
    assert_eq!(fs::read(&fixture.docker_config).unwrap(), changed);
}

#[test]
fn non_interactive_configuration_requires_confirmation() {
    let fixture = Fixture::new();
    let original = fs::read(&fixture.docker_config).unwrap();
    let output = fixture
        .secretspec()
        .args([
            "docker",
            "configure",
            "--registry",
            "ghcr.io",
            "--token-secret",
            "DOCKER_TOKEN",
            "--username",
            "registry-user",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("pass --yes"));
    assert_eq!(fs::read(&fixture.docker_config).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn configuration_preserves_a_symlinked_docker_config() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let target = fixture.docker_config.with_file_name("actual-config.json");
    fs::rename(&fixture.docker_config, &target).unwrap();
    symlink(&target, &fixture.docker_config).unwrap();
    let original = read_json(&target);

    assert_success("docker configure", &fixture.configure("ghcr.io"));
    assert!(
        fs::symlink_metadata(&fixture.docker_config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(read_json(&target)["credHelpers"]["ghcr.io"], "secretspec");

    let output = fixture
        .secretspec()
        .args(["docker", "unconfigure", "--registry", "ghcr.io", "--yes"])
        .output()
        .unwrap();
    assert_success("docker unconfigure", &output);
    assert!(
        fs::symlink_metadata(&fixture.docker_config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(read_json(&target), original);
}

#[test]
fn helper_is_read_only() {
    let fixture = Fixture::new();
    for (operation, input) in [
        (
            "store",
            br#"{"ServerURL":"ghcr.io","Username":"user","Secret":"secret"}"#.as_slice(),
        ),
        ("erase", b"ghcr.io".as_slice()),
    ] {
        let output = fixture.helper(operation, input);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("read-only"));
    }
}
