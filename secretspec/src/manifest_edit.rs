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
