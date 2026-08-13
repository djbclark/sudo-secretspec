use std::path::Path;

use sudo_secretspec_cli::config::Config;

fn valid() -> &'static str {
    r#"
engine = "/usr/local/libexec/sudo-secretspec-engine"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#
}

#[test]
fn parses_typed_toml_without_shell_evaluation() {
    let config = Config::parse(valid()).unwrap();
    assert_eq!(config.vault(), Path::new("/var/db/sudo-secretspec"));
    assert_eq!(config.service_user(), "_sudo_secretspec");
}

#[test]
fn rejects_unknown_keys_and_noncanonical_vaults() {
    let unknown = format!("{}\ncommand = \"touch /tmp/pwned\"\n", valid());
    assert!(Config::parse(&unknown).is_err());

    let nested = valid().replace(
        "vault = \"/var/db/sudo-secretspec\"",
        "vault = \"/var/db/team/sudo-secretspec\"",
    );
    assert!(Config::parse(&nested).is_err());

    let mismatched = valid().replace("/private/var/db/sudo-secretspec", "/private/var/db/other");
    assert!(Config::parse(&mismatched).is_err());
}

#[test]
fn rejects_invalid_service_identity_and_relative_paths() {
    assert!(Config::parse(&valid().replace("_sudo_secretspec", "root")).is_err());
    assert!(
        Config::parse(&valid().replace(
            "/usr/local/libexec/sudo-secretspec-engine",
            "./secretspec-engine",
        ))
        .is_err()
    );
}
