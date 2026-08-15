//! Declaration edits to a `secretspec.toml` source string.
//!
//! Split out of the `cli` module so a caller can perform the edit without
//! taking on the interactive CLI's dependency surface. The downstream
//! privilege boundary needs exactly this and nothing else: pulling in `cli`
//! would put `inquire`, `clap` and friends inside a root-privileged broker.
//!
//! Everything here is pure — it takes manifest text and returns manifest
//! text. Writing is the caller's problem, deliberately: the boundary must
//! write in place to preserve the vault file's ownership, while the CLI
//! replaces its manifest atomically via a temporary file.

use miette::{IntoDiagnostic, Result, WrapErr, miette};

pub(crate) fn validate_add_secret_name(name: &str) -> Result<()> {
    if !crate::config::is_valid_identifier(name) {
        return Err(miette!(
            "Invalid secret name '{}': must be a valid identifier (alphanumeric and underscores, not starting with a number)",
            name
        ));
    }
    // `Profile` reserves this key for its defaults table before flattening all
    // remaining keys into secret declarations. Without an explicit check, the
    // edit would be valid TOML but would not actually declare a secret.
    if name == "defaults" {
        return Err(miette!(
            "Secret name 'defaults' is reserved for profile defaults"
        ));
    }
    Ok(())
}

/// Insert a secret declaration into a `secretspec.toml` source string.
///
/// Pure: takes the manifest text and returns the edited text, touching no
/// filesystem. Exposed so the downstream privilege boundary can perform the
/// same edit without shelling out to this CLI — it must write the result
/// itself, in place, to preserve the vault file's ownership.
pub fn add_secret_to_manifest(
    source: &str,
    profile: &str,
    name: &str,
    description: &str,
) -> Result<String> {
    use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

    validate_add_secret_name(name)?;
    if description.trim().is_empty() {
        return Err(miette!("Secret description cannot be empty"));
    }

    let mut doc = source
        .parse::<DocumentMut>()
        .into_diagnostic()
        .wrap_err("Failed to parse secretspec.toml for editing")?;
    let profiles = doc
        .get_mut("profiles")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| miette!("secretspec.toml does not contain a [profiles] table"))?;

    if !profiles.contains_key(profile) {
        profiles.insert(profile, Item::Table(Table::new()));
    }
    let profile_table = profiles
        .get_mut(profile)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| miette!("Profile '{}' is not a TOML table", profile))?;

    if profile_table.contains_key(name) {
        return Err(miette!(
            "Secret '{}' is already declared in profile '{}'",
            name,
            profile
        ));
    }

    let mut secret = InlineTable::new();
    secret.insert("description", Value::from(description));
    profile_table.insert(name, toml_edit::value(secret));

    Ok(doc.to_string())
}

/// Whether `source` declares `name` in `profile`.
///
/// Parsed, not searched. A substring test would match the name inside a comment
/// or another secret's description, and the inverse mistake is worse because it
/// is silent — a caller relying on this to protect tracked declarations would
/// get a wrong answer with no indication.
///
/// A malformed manifest is an `Err`, never a `false`. Callers using this as a
/// guard must fail closed, and that decision belongs to them rather than being
/// smuggled in here as a default: "I could not parse it" is not "the name is
/// absent from it".
pub fn declares_secret(source: &str, profile: &str, name: &str) -> Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let doc = source
        .parse::<DocumentMut>()
        .into_diagnostic()
        .wrap_err("Failed to parse secretspec.toml")?;
    Ok(doc
        .get("profiles")
        .and_then(Item::as_table_like)
        .and_then(|profiles| profiles.get(profile))
        .and_then(Item::as_table_like)
        .is_some_and(|table| table.contains_key(name)))
}

/// Remove a secret declaration from a `secretspec.toml` source string.
///
/// The inverse of [`add_secret_to_manifest`], and pure for the same reason.
///
/// `toml_edit` preserves the formatting of everything it does not touch, so
/// removing a declaration that `add_secret_to_manifest` inserted restores the
/// original text byte for byte. That matters more than it looks: the downstream
/// boundary's `template-check` compares the runtime manifest against the tracked
/// template as raw bytes, so "undo" has to mean *byte-identical*, not merely
/// semantically equivalent.
///
/// Removing a name that is not declared is an error rather than a silent no-op.
/// A caller undeclaring something already absent has a wrong model of the
/// manifest, and saying so is cheaper than letting them believe they cleaned up
/// state that was never there.
pub fn remove_secret_from_manifest(source: &str, profile: &str, name: &str) -> Result<String> {
    use toml_edit::{DocumentMut, Item};

    validate_add_secret_name(name)?;

    let mut doc = source
        .parse::<DocumentMut>()
        .into_diagnostic()
        .wrap_err("Failed to parse secretspec.toml for editing")?;
    let profiles = doc
        .get_mut("profiles")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| miette!("secretspec.toml does not contain a [profiles] table"))?;

    let profile_table = profiles
        .get_mut(profile)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| miette!("Profile '{}' is not declared in this manifest", profile))?;

    if profile_table.remove(name).is_none() {
        return Err(miette!(
            "Secret '{}' is not declared in profile '{}'",
            name,
            profile
        ));
    }

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"[project]
name = "demo"
revision = "1.0"

[profiles.default]
EXISTING = { description = "already here" }
"#;

    #[test]
    fn removing_an_added_declaration_restores_the_original_bytes() {
        // The property the boundary depends on. `template-check` compares the
        // runtime manifest against the tracked template as raw BYTES, so an
        // "undo" that is merely semantically equivalent still reports drift
        // forever. toml_edit preserves untouched formatting; this proves it.
        let added = add_secret_to_manifest(MANIFEST, "default", "SCRATCH", "temp").unwrap();
        assert_ne!(added, MANIFEST, "add did not change anything");

        let removed = remove_secret_from_manifest(&added, "default", "SCRATCH").unwrap();

        assert_eq!(removed, MANIFEST);
    }

    #[test]
    fn removing_a_declaration_that_is_not_there_is_an_error() {
        let err = remove_secret_from_manifest(MANIFEST, "default", "ABSENT").unwrap_err();
        assert!(err.to_string().contains("not declared"), "{err}");
    }

    #[test]
    fn removing_leaves_sibling_declarations_alone() {
        let added = add_secret_to_manifest(MANIFEST, "default", "SCRATCH", "temp").unwrap();
        let removed = remove_secret_from_manifest(&added, "default", "SCRATCH").unwrap();
        assert!(removed.contains("EXISTING"));
    }

    #[test]
    fn declares_secret_sees_a_real_declaration() {
        assert!(declares_secret(MANIFEST, "default", "EXISTING").unwrap());
        assert!(!declares_secret(MANIFEST, "default", "ABSENT").unwrap());
    }

    #[test]
    fn declares_secret_is_not_fooled_by_a_comment_or_a_description() {
        // Why this is parsed rather than searched. Both of these contain the
        // name as text while declaring nothing of the sort.
        let manifest = r#"[project]
name = "demo"

[profiles.default]
# TODO: declare LOOKALIKE next release
OTHER = { description = "unrelated, mentions LOOKALIKE in prose" }
"#;
        assert!(!declares_secret(manifest, "default", "LOOKALIKE").unwrap());
    }

    #[test]
    fn declares_secret_reports_an_unparseable_manifest_rather_than_false() {
        // A guard built on this must fail closed, which it cannot do if a
        // broken file is indistinguishable from an absent name.
        assert!(declares_secret("this is not toml {{{", "default", "ANY").is_err());
    }

    #[test]
    fn declares_secret_is_profile_scoped() {
        let manifest = r#"[project]
name = "demo"

[profiles.default]
ONLY_DEFAULT = { description = "d" }

[profiles.production]
ONLY_PROD = { description = "p" }
"#;
        assert!(declares_secret(manifest, "default", "ONLY_DEFAULT").unwrap());
        assert!(!declares_secret(manifest, "production", "ONLY_DEFAULT").unwrap());
    }
}
