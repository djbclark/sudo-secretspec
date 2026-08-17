//! Local SQLite provider (0.20+).
//!
//! Stores each secret as one row in a local SQLite database. Convention
//! addresses are isolated by project and profile at `{project}/{profile}/{key}`;
//! a native `ref` supplies that key directly as `item`.
//!
//! Confidentiality comes entirely from filesystem permissions on the database
//! file — the schema and connection carry no credential of their own. That is
//! what lets identical code serve both an ordinary user-owned store and a
//! privilege-boundary vault file guarded by a `0600` service-user-owned path:
//! the privilege model stays external to the provider.

use super::{Address, Provider, ProviderUrl};
use crate::config::{NativeAddress, expand_tilde};
use crate::{Result, SecretSpecError};
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Configuration for the local SQLite provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    /// Path to the SQLite database file.
    pub path: PathBuf,
}

impl TryFrom<&ProviderUrl> for SqliteConfig {
    type Error = SecretSpecError;

    fn try_from(url: &ProviderUrl) -> Result<Self> {
        if url.scheme() != "sqlite" {
            return Err(operation_error(format!(
                "Invalid scheme '{}' for sqlite provider",
                url.scheme()
            )));
        }
        if !url.username().is_empty() || url.password().is_some() || url.has_query() {
            return Err(operation_error(
                "sqlite provider URIs take only a database path; user information and query \
                 options are not supported",
            ));
        }

        let uri_path = url.path();
        let path = match url.host() {
            Some(host) => format!("{host}{uri_path}"),
            None => uri_path,
        };
        if path.is_empty() || path == "/" {
            return Err(operation_error(
                "No SQLite database path given. Use sqlite:./secrets.db or \
                 sqlite:/absolute/path/secrets.db.",
            ));
        }

        Ok(Self {
            path: PathBuf::from(path),
        })
    }
}

/// A provider backed by a local SQLite database, one row per secret.
pub struct SqliteProvider {
    config: SqliteConfig,
}

crate::register_provider! {
    struct: SqliteProvider,
    config: SqliteConfig,
    name: "sqlite",
    description: "Local SQLite database (0.20+)",
    schemes: ["sqlite"],
    examples: ["sqlite:./secrets.db", "sqlite:///var/lib/secrets.db"],
    deletes: true,
}

impl SqliteProvider {
    pub fn new(mut config: SqliteConfig) -> Self {
        config.path = expand_tilde(config.path);
        Self { config }
    }

    #[cfg(unix)]
    fn create_private_directory(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;

        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }

    #[cfg(not(unix))]
    fn create_private_directory(path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    #[cfg(unix)]
    fn restrict_file(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
            operation_error(format!(
                "failed to restrict permissions for sqlite database '{}': {error}",
                path.display()
            ))
        })
    }

    #[cfg(not(unix))]
    fn restrict_file(_path: &Path) -> Result<()> {
        Ok(())
    }

    /// Opens the database, creating its parent directory and schema on first
    /// use. Every open re-asserts `0600` permissions on the file, in case it
    /// pre-existed with looser ones.
    fn connection(&self) -> Result<Connection> {
        if let Some(parent) = self.config.path.parent()
            && !parent.as_os_str().is_empty()
        {
            Self::create_private_directory(parent).map_err(|error| {
                operation_error(format!(
                    "failed to create sqlite provider directory '{}': {error}",
                    parent.display()
                ))
            })?;
        }

        let conn = Connection::open(&self.config.path).map_err(|error| {
            operation_error(format!(
                "failed to open sqlite database '{}': {error}",
                self.config.path.display()
            ))
        })?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(|error| {
            operation_error(format!(
                "failed to configure sqlite database '{}': {error}",
                self.config.path.display()
            ))
        })?;
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE;\
             PRAGMA synchronous=FULL;\
             PRAGMA trusted_schema=OFF;\
             CREATE TABLE IF NOT EXISTS secrets (\
                 item TEXT PRIMARY KEY,\
                 value TEXT NOT NULL\
             ) STRICT;",
        )
        .map_err(|error| {
            operation_error(format!(
                "failed to prepare sqlite database '{}': {error}",
                self.config.path.display()
            ))
        })?;
        Self::restrict_file(&self.config.path)?;
        Ok(conn)
    }
}

impl Provider for SqliteProvider {
    fn convention_address(&self, project: &str, profile: &str, key: &str) -> Result<NativeAddress> {
        Ok(NativeAddress {
            item: format!("{project}/{profile}/{key}"),
            ..Default::default()
        })
    }

    fn get(&self, addr: Address<'_>) -> Result<Option<SecretString>> {
        let item = super::flat_item(self, addr)?;
        if !self.config.path.exists() {
            return Ok(None);
        }
        let conn = self.connection()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM secrets WHERE item = ?1",
                params![item.as_ref()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                operation_error(format!(
                    "failed to read sqlite provider entry '{item}': {error}"
                ))
            })?;
        Ok(value.map(|value| SecretString::new(value.into())))
    }

    fn set(&self, addr: Address<'_>, value: &SecretString) -> Result<()> {
        self.check_writable(addr)?;
        let item = super::flat_item(self, addr)?;
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO secrets (item, value) VALUES (?1, ?2) \
             ON CONFLICT(item) DO UPDATE SET value = excluded.value",
            params![item.as_ref(), value.expose_secret()],
        )
        .map_err(|error| {
            operation_error(format!(
                "failed to write sqlite provider entry '{item}': {error}"
            ))
        })?;
        Ok(())
    }

    fn delete(&self, addr: Address<'_>) -> Result<bool> {
        let item = super::flat_item(self, addr)?;
        if !self.config.path.exists() {
            return Ok(false);
        }
        let conn = self.connection()?;
        let changed = conn
            .execute(
                "DELETE FROM secrets WHERE item = ?1",
                params![item.as_ref()],
            )
            .map_err(|error| {
                operation_error(format!(
                    "failed to delete sqlite provider entry '{item}': {error}"
                ))
            })?;
        Ok(changed > 0)
    }

    fn supports_delete(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        Self::PROVIDER_NAME
    }

    fn uri(&self) -> String {
        format!(
            "sqlite:{}",
            ProviderUrl::encode(&self.config.path.display().to_string())
        )
    }

    fn physical_store_path(&self) -> Option<&Path> {
        Some(&self.config.path)
    }

    fn with_base_dir(&mut self, base_dir: &Path) {
        if self.config.path.is_relative() {
            self.config.path = base_dir.join(&self.config.path);
        }
    }

    fn get_many(&self, requests: &[(&str, Address<'_>)]) -> Result<HashMap<String, SecretString>> {
        // Resolve every address up front so an invalid coordinate is rejected
        // the same way whether or not the database file exists yet.
        let mut items = Vec::with_capacity(requests.len());
        for (name, addr) in requests {
            items.push((*name, super::flat_item(self, *addr)?));
        }
        if !self.config.path.exists() {
            return Ok(HashMap::new());
        }
        let conn = self.connection()?;
        let mut statement = conn
            .prepare("SELECT value FROM secrets WHERE item = ?1")
            .map_err(|error| {
                operation_error(format!("failed to query sqlite database: {error}"))
            })?;

        let mut results = HashMap::new();
        for (name, item) in items {
            let value: Option<String> = statement
                .query_row(params![item.as_ref()], |row| row.get(0))
                .optional()
                .map_err(|error| {
                    operation_error(format!(
                        "failed to read sqlite provider entry '{item}': {error}"
                    ))
                })?;
            if let Some(value) = value {
                results.insert(name.to_string(), SecretString::new(value.into()));
            }
        }
        Ok(results)
    }
}

fn operation_error(message: impl Into<String>) -> SecretSpecError {
    SecretSpecError::ProviderOperationFailed(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use url::Url;

    fn provider_url(value: &str) -> ProviderUrl {
        ProviderUrl::new(Url::parse(value).unwrap())
    }

    fn provider(path: PathBuf) -> SqliteProvider {
        SqliteProvider::new(SqliteConfig { path })
    }

    fn convention<'a>(key: &'a str) -> Address<'a> {
        Address::convention("project", "production", key)
    }

    #[test]
    fn config_parses_relative_and_absolute_paths() {
        let relative = SqliteConfig::try_from(&provider_url("sqlite://./secrets.db")).unwrap();
        assert_eq!(relative.path, PathBuf::from("./secrets.db"));

        let absolute =
            SqliteConfig::try_from(&provider_url("sqlite:///var/lib/secrets.db")).unwrap();
        assert_eq!(absolute.path, PathBuf::from("/var/lib/secrets.db"));
    }

    #[test]
    fn config_rejects_missing_path_and_query_options() {
        for uri in [
            "sqlite://",
            "sqlite://./secrets.db?history=on",
            "sqlite://user:pass@./secrets.db",
        ] {
            assert!(SqliteConfig::try_from(&provider_url(uri)).is_err(), "{uri}");
        }
    }

    #[test]
    fn registry_builds_documented_uri() {
        let provider = Box::<dyn Provider>::try_from("sqlite:./secrets.db").unwrap();
        assert_eq!(provider.name(), "sqlite");
    }

    #[test]
    fn convention_round_trip_is_exact_and_profile_isolated() {
        let temp = TempDir::new().unwrap();
        let provider = provider(temp.path().join("secrets.db"));
        let value = SecretString::new("s3cr3t\nvalue".to_string().into());

        provider
            .set(Address::convention("app", "development", "TOKEN"), &value)
            .unwrap();

        let found = provider
            .get(Address::convention("app", "development", "TOKEN"))
            .unwrap()
            .unwrap();
        assert_eq!(found.expose_secret(), value.expose_secret());
        assert!(
            provider
                .get(Address::convention("app", "production", "TOKEN"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn set_updates_existing_row() {
        let temp = TempDir::new().unwrap();
        let provider = provider(temp.path().join("secrets.db"));
        provider
            .set(convention("KEY"), &SecretString::new("old".into()))
            .unwrap();
        provider
            .set(convention("KEY"), &SecretString::new("new".into()))
            .unwrap();
        assert_eq!(
            provider
                .get(convention("KEY"))
                .unwrap()
                .unwrap()
                .expose_secret(),
            "new"
        );
    }

    #[test]
    fn missing_database_and_entry_are_absent_without_creating_a_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("secrets.db");
        assert!(
            provider(path.clone())
                .get(convention("MISSING"))
                .unwrap()
                .is_none()
        );
        assert!(!path.exists());
    }

    #[test]
    fn delete_is_idempotent_and_does_not_create_a_missing_database() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("secrets.db");
        let provider = provider(path.clone());
        assert!(!provider.delete(convention("KEY")).unwrap());
        assert!(!path.exists());

        provider
            .set(convention("KEY"), &SecretString::new("value".into()))
            .unwrap();
        assert!(provider.delete(convention("KEY")).unwrap());
        assert!(!provider.delete(convention("KEY")).unwrap());
        assert!(provider.get(convention("KEY")).unwrap().is_none());
    }

    #[test]
    fn uri_round_trips() {
        let provider = provider(PathBuf::from("./my secrets.db"));
        let uri = provider.uri();
        assert_eq!(uri, "sqlite:./my%20secrets.db");
        let reparsed = Box::<dyn Provider>::try_from(uri.as_str()).unwrap();
        assert_eq!(reparsed.uri(), uri);
    }

    #[test]
    fn relative_path_is_rebased_to_the_manifest() {
        let mut provider = provider(PathBuf::from("data/secrets.db"));
        provider.with_base_dir(Path::new("/project"));
        assert_eq!(
            provider.config.path,
            PathBuf::from("/project/data/secrets.db")
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("secrets.db");
        provider(path.clone())
            .set(convention("KEY"), &SecretString::new("value".into()))
            .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn get_many_reads_multiple_entries_and_omits_missing_ones() {
        let temp = TempDir::new().unwrap();
        let provider = provider(temp.path().join("secrets.db"));
        provider
            .set(convention("ONE"), &SecretString::new("one".into()))
            .unwrap();
        provider
            .set(convention("TWO"), &SecretString::new("two".into()))
            .unwrap();

        let results = provider
            .get_many(&[
                ("FIRST", convention("ONE")),
                ("SECOND", convention("TWO")),
                ("MISSING", convention("THREE")),
            ])
            .unwrap();
        assert_eq!(results["FIRST"].expose_secret(), "one");
        assert_eq!(results["SECOND"].expose_secret(), "two");
        assert!(!results.contains_key("MISSING"));
    }

    #[test]
    fn concurrent_writes_preserve_both_entries() {
        let temp = TempDir::new().unwrap();
        let provider = std::sync::Arc::new(provider(temp.path().join("secrets.db")));
        std::thread::scope(|scope| {
            for (name, value) in [("ONE", "one"), ("TWO", "two")] {
                let provider = std::sync::Arc::clone(&provider);
                scope.spawn(move || {
                    provider
                        .set(convention(name), &SecretString::new(value.into()))
                        .unwrap();
                });
            }
        });
        assert_eq!(
            provider
                .get(convention("ONE"))
                .unwrap()
                .unwrap()
                .expose_secret(),
            "one"
        );
        assert_eq!(
            provider
                .get(convention("TWO"))
                .unwrap()
                .unwrap()
                .expose_secret(),
            "two"
        );
    }

    #[test]
    fn native_item_addresses_an_arbitrary_row() {
        let temp = TempDir::new().unwrap();
        let provider = provider(temp.path().join("secrets.db"));
        let address = NativeAddress {
            item: "shared/anything".to_string(),
            ..Default::default()
        };
        provider
            .set(
                Address::Native(&address),
                &SecretString::new("value".into()),
            )
            .unwrap();
        assert_eq!(
            provider
                .get(Address::Native(&address))
                .unwrap()
                .unwrap()
                .expose_secret(),
            "value"
        );
    }

    #[test]
    fn unsupported_native_coordinate_is_rejected_even_before_the_database_exists() {
        // Regression: `get`/`delete` used to check file existence before
        // resolving the address, so an invalid coordinate was misreported as
        // "not found" whenever the store had not been created yet.
        let temp = TempDir::new().unwrap();
        let provider = provider(temp.path().join("secrets.db"));
        let address = NativeAddress {
            item: "KEY".to_string(),
            field: Some("password".to_string()),
            ..Default::default()
        };
        let error = provider.get(Address::Native(&address)).unwrap_err();
        assert!(error.to_string().contains("`field`"), "{error}");
        let error = provider.delete(Address::Native(&address)).unwrap_err();
        assert!(error.to_string().contains("`field`"), "{error}");
    }
}
