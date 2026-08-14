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
fn a_config_without_a_profile_still_parses_and_pins_the_default() {
    // `deny_unknown_fields` tolerates a new key but not a missing one, so this
    // is what lets an older installer's config be read by a newer broker.
    // Without the default it would be a hard parse failure on upgrade.
    assert!(!valid().contains("profile"));
    assert_eq!(Config::parse(valid()).unwrap().profile, "default");
}

#[test]
fn an_explicit_profile_is_taken_from_the_protected_config() {
    let pinned = format!("{}profile = \"production\"\n", valid());
    assert_eq!(Config::parse(&pinned).unwrap().profile, "production");
}

#[test]
fn rejects_profile_names_that_are_not_plain_manifest_keys() {
    // A profile name reaches the engine as a table key, never as a path. These
    // are the shapes that would mean something else if it ever did.
    for bad in [
        "",
        "../other",
        "a/b",
        "with space",
        "-leading",
        "9leading",
        "sémantique",
    ] {
        let cfg = format!("{}profile = \"{bad}\"\n", valid());
        assert!(
            Config::parse(&cfg).is_err(),
            "profile {bad:?} must be rejected"
        );
    }
    for good in ["default", "production", "staging-2", "ci_runner", "A1"] {
        let cfg = format!("{}profile = \"{good}\"\n", valid());
        assert!(
            Config::parse(&cfg).is_ok(),
            "profile {good:?} must be accepted"
        );
    }
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
