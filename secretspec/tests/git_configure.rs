use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
    manifest: PathBuf,
    global_config: PathBuf,
    path: OsString,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        let repository = root.join("repository");
        fs::create_dir(&repository).unwrap();
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repository)
            .env("HOME", &root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert_success("git init", &output);
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=SecretSpec Test",
                "-c",
                "user.email=secretspec@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "--quiet",
                "-m",
                "initial",
            ])
            .current_dir(&repository)
            .env("HOME", &root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert_success("git commit", &output);

        let manifest = repository.join("secretspec.toml");
        fs::write(
            &manifest,
            r#"
[project]
name = "git-configure"
revision = "1.0"
require_reason = false

[profiles.default]
GITHUB_TOKEN = { description = "GitHub token", default = "token=value", providers = ["null"] }
"#,
        )
        .unwrap();

        let binary = Path::new(env!("CARGO_BIN_EXE_secretspec"));
        let helper = Path::new(env!("CARGO_BIN_EXE_git-credential-secretspec"));
        assert_eq!(binary.parent(), helper.parent());
        let existing_path = env::var_os("PATH").unwrap_or_default();
        let path = env::join_paths(
            std::iter::once(binary.parent().unwrap().to_path_buf())
                .chain(env::split_paths(&existing_path)),
        )
        .unwrap();

        Self {
            _temp: temp,
            root: root.clone(),
            repository,
            manifest,
            global_config: root.join("global.gitconfig"),
            path,
        }
    }

    fn command(&self) -> Command {
        self.command_in(&self.repository)
    }

    fn command_in(&self, directory: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_secretspec"));
        command
            .arg("--file")
            .arg(&self.manifest)
            .current_dir(directory)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("PATH", &self.path)
            .env_remove("SECRETSPEC_FILE")
            .env_remove("SECRETSPEC_PROFILE")
            .env_remove("SECRETSPEC_PROVIDER")
            .env_remove("SECRETSPEC_REASON");
        command
    }

    fn git(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.repository)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("PATH", &self.path)
            .output()
            .unwrap()
    }

    fn git_ok(&self, args: &[&str]) -> String {
        let output = self.git(args);
        assert_success("git", &output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn credential_fill(&self, request: &[u8]) -> Output {
        let mut child = Command::new("git")
            .args(["credential", "fill"])
            .current_dir(&self.repository)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("PATH", &self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(request).unwrap();
        child.wait_with_output().unwrap()
    }

    fn configure_args(global: bool) -> Vec<&'static str> {
        let mut args = vec![
            "git",
            "configure",
            "--url",
            "https://github.com",
            "--token-secret",
            "GITHUB_TOKEN",
            "--username",
            "vimjoyer",
        ];
        if global {
            args.extend(["--global", "--yes"]);
        }
        args
    }
}

#[test]
fn local_configuration_can_be_removed_from_a_linked_worktree() {
    let fixture = Fixture::new();
    fixture.git_ok(&["config", "--local", "user.name", "Existing User"]);
    let config_path = fixture.git_ok(&["rev-parse", "--git-path", "config"]);
    let config_path = fixture.repository.join(config_path.trim());
    let original_config = fs::read(&config_path).unwrap();
    let worktree = fixture.root.join("linked-worktree");
    let output = fixture.git(&[
        "worktree",
        "add",
        "--detach",
        worktree.to_str().unwrap(),
        "HEAD",
    ]);
    assert_success("git worktree add", &output);

    let output = fixture
        .command()
        .args(Fixture::configure_args(false))
        .output()
        .unwrap();
    assert_success("local configure", &output);
    let includes = fixture.git_ok(&["config", "--local", "--get-all", "include.path"]);
    let managed_path = PathBuf::from(includes.lines().next().unwrap());
    assert!(managed_path.exists());

    let output = fixture
        .command_in(&worktree)
        .args(["git", "unconfigure", "--all"])
        .output()
        .unwrap();
    assert_success("linked-worktree unconfigure all", &output);
    assert!(!managed_path.exists());
    assert_eq!(fs::read(&config_path).unwrap(), original_config);
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

#[test]
fn local_configure_works_and_unconfigure_restores_existing_config() {
    let fixture = Fixture::new();
    fixture.git_ok(&["config", "--local", "user.name", "Existing User"]);
    fixture.git_ok(&["config", "--local", "credential.helper", "!true"]);
    fixture.git_ok(&[
        "config",
        "--local",
        "credential.https://github.com.username",
        "existing-user",
    ]);
    fixture.git_ok(&[
        "config",
        "--local",
        "include.path",
        "/tmp/unrelated-git-config",
    ]);
    let config_path = fixture.git_ok(&["rev-parse", "--git-path", "config"]);
    let config_path = fixture.repository.join(config_path.trim());
    let original_config = fs::read(&config_path).unwrap();

    let output = fixture
        .command()
        .args(Fixture::configure_args(false))
        .output()
        .unwrap();
    assert_success("local configure", &output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Undo with: secretspec git unconfigure"));
    assert!(!stdout.contains("token=value"));

    let includes = fixture.git_ok(&["config", "--local", "--get-all", "include.path"]);
    assert!(includes.contains("/tmp/unrelated-git-config"));
    let managed_path = includes
        .lines()
        .map(PathBuf::from)
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == "secretspec-credentials")
        })
        .unwrap();
    assert!(managed_path.exists());

    let output = fixture
        .command()
        .args(Fixture::configure_args(false))
        .output()
        .unwrap();
    assert_success("repeated local configure", &output);
    let includes = fixture.git_ok(&["config", "--local", "--get-all", "include.path"]);
    assert_eq!(
        includes
            .lines()
            .filter(|line| Path::new(line) == managed_path)
            .count(),
        1
    );

    let fill = fixture.credential_fill(b"protocol=https\nhost=github.com\n\n");
    assert_success("git credential fill", &fill);
    let filled = String::from_utf8(fill.stdout).unwrap();
    assert!(filled.contains("username=vimjoyer\n"));
    assert!(filled.contains("password=token=value\n"));

    let output = fixture
        .command()
        .args(["git", "unconfigure", "--url", "https://github.com"])
        .output()
        .unwrap();
    assert_success("local unconfigure", &output);
    assert!(!managed_path.exists());
    assert_eq!(fs::read(&config_path).unwrap(), original_config);

    let output = fixture
        .command()
        .args([
            "git",
            "configure",
            "--url",
            "https://github.com/cachix",
            "--token-secret",
            "GITHUB_TOKEN",
            "--username",
            "vimjoyer",
        ])
        .output()
        .unwrap();
    assert_success("path-scoped local configure", &output);
    let fill =
        fixture.credential_fill(b"protocol=https\nhost=github.com\npath=cachix/secretspec\n\n");
    assert_success("path-scoped git credential fill", &fill);
    let filled = String::from_utf8(fill.stdout).unwrap();
    assert!(filled.contains("username=vimjoyer\n"));
    assert!(filled.contains("password=token=value\n"));

    let output = fixture
        .command()
        .args(["git", "unconfigure", "--all"])
        .output()
        .unwrap();
    assert_success("local unconfigure all", &output);
    assert_eq!(fs::read(&config_path).unwrap(), original_config);
}

#[test]
fn global_changes_require_confirmation_and_unconfigure_all_restores_config() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.global_config,
        "[user]\n\tname = Existing User\n[credential]\n\thelper = !true\n",
    )
    .unwrap();
    let original_config = fs::read(&fixture.global_config).unwrap();

    let output = fixture
        .command()
        .args([
            "git",
            "configure",
            "--url",
            "https://github.com",
            "--token-secret",
            "GITHUB_TOKEN",
            "--global",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("confirmation")
            && String::from_utf8_lossy(&output.stderr).contains("--yes"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&fixture.global_config).unwrap(), original_config);

    let output = fixture
        .command()
        .args(Fixture::configure_args(true))
        .output()
        .unwrap();
    assert_success("global configure", &output);
    let output = fixture
        .command()
        .args([
            "git",
            "configure",
            "--url",
            "https://gitlab.com",
            "--token-secret",
            "GITHUB_TOKEN",
            "--global",
            "--yes",
        ])
        .output()
        .unwrap();
    assert_success("second global configure", &output);

    let fill = fixture.credential_fill(b"protocol=https\nhost=github.com\n\n");
    assert_success("global git credential fill", &fill);
    let filled = String::from_utf8(fill.stdout).unwrap();
    assert!(filled.contains("username=vimjoyer\n"));
    assert!(filled.contains("password=token=value\n"));

    let includes = fixture.git_ok(&["config", "--global", "--get-all", "include.path"]);
    let managed_path = PathBuf::from(includes.lines().next().unwrap());
    assert!(managed_path.exists());
    let configured_config = fs::read(&fixture.global_config).unwrap();
    let configured_managed = fs::read(&managed_path).unwrap();

    let output = fixture
        .command()
        .args(["git", "unconfigure", "--all", "--global"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("confirmation")
            && String::from_utf8_lossy(&output.stderr).contains("--yes")
    );
    assert_eq!(fs::read(&fixture.global_config).unwrap(), configured_config);
    assert_eq!(fs::read(&managed_path).unwrap(), configured_managed);

    let output = fixture
        .command()
        .args(["git", "unconfigure", "--all", "--global", "--yes"])
        .output()
        .unwrap();
    assert_success("global unconfigure all", &output);
    assert!(!managed_path.exists());
    assert_eq!(fs::read(&fixture.global_config).unwrap(), original_config);
}
