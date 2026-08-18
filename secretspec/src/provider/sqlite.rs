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
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Configuration for the local SQLite provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    /// Path to the SQLite database file.
    pub path: PathBuf,
    /// Whether history tracking is enabled.
    #[serde(default)]
    pub history: bool,
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
        if !url.username().is_empty() || url.password().is_some() {
            return Err(operation_error(
                "sqlite provider URIs take only a database path and an optional '?history=true' parameter",
            ));
        }
        let mut history = false;
        if url.has_query() {
            let pairs: Vec<_> = url.query_pairs().collect();
            if pairs.len() != 1 || pairs[0].0 != "history" {
                return Err(operation_error(
                    "sqlite provider URIs take only a database path and an optional '?history=true' parameter",
                ));
            }
            match pairs[0].1.as_ref() {
                "true" | "1" | "yes" | "on" => history = true,
                "false" | "0" | "no" | "off" => history = false,
                _ => return Err(operation_error("invalid value for 'history' parameter")),
            }
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
            history,
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
        let mut init_sql = String::from(
            // `foreign_keys` is off by default and is a *per-connection* setting,
            // so without this the REFERENCES clauses on `captured_values` below
            // are documentation rather than a constraint: a tombstone could name
            // a `destroyed_by` sequence no entry has. Every connection this
            // provider hands out comes through here, which is what makes one
            // line sufficient.
            // `secure_delete` is also per-connection, and it is *off* in the
            // library this crate links (Homebrew SQLite 3.53.4 reports 0).
            // Without it SQLite unlinks a deleted row from the b-tree but
            // leaves its bytes on the freed page, so `delete` — and the
            // tombstoning `destroy` builds on it — left every plaintext value
            // recoverable by reading the file. Setting it here makes freed
            // content zeroed on the way out. It is not retroactive: residue
            // already on freelist pages needs a `VACUUM`.
            "PRAGMA foreign_keys=ON;\
             PRAGMA secure_delete=ON;\
             PRAGMA journal_mode=DELETE;\
             PRAGMA synchronous=FULL;\
             PRAGMA trusted_schema=OFF;\
             CREATE TABLE IF NOT EXISTS secrets (\
                 item TEXT PRIMARY KEY,\
                 value TEXT NOT NULL\
             ) STRICT;",
        );
        if self.config.history {
            init_sql.push_str(HISTORY_SCHEMA);
        }
        conn.execute_batch(&init_sql).map_err(|error| {
            operation_error(format!(
                "failed to prepare sqlite database '{}': {error}",
                self.config.path.display()
            ))
        })?;
        if self.config.history {
            migrate_history(&conn)?;
        }
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
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| {
            operation_error(format!("failed to start sqlite transaction: {error}"))
        })?;
        tx.execute(
            "INSERT INTO secrets (item, value) VALUES (?1, ?2) \
             ON CONFLICT(item) DO UPDATE SET value = excluded.value",
            params![item.as_ref(), value.expose_secret()],
        )
        .map_err(|error| {
            operation_error(format!(
                "failed to write sqlite provider entry '{item}': {error}"
            ))
        })?;
        if self.config.history {
            capture_history(&tx, "set")?;
        }
        tx.commit().map_err(|error| {
            operation_error(format!("failed to commit sqlite transaction: {error}"))
        })?;
        Ok(())
    }

    fn delete(&self, addr: Address<'_>) -> Result<bool> {
        let item = super::flat_item(self, addr)?;
        if !self.config.path.exists() {
            return Ok(false);
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| {
            operation_error(format!("failed to start sqlite transaction: {error}"))
        })?;
        let changed = tx
            .execute(
                "DELETE FROM secrets WHERE item = ?1",
                params![item.as_ref()],
            )
            .map_err(|error| {
                operation_error(format!(
                    "failed to delete sqlite provider entry '{item}': {error}"
                ))
            })?;
        if changed > 0 && self.config.history {
            capture_history(&tx, "delete")?;
        }
        tx.commit().map_err(|error| {
            operation_error(format!("failed to commit sqlite transaction: {error}"))
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

// ---------------------------------------------------------------------------
// History Capture
// ---------------------------------------------------------------------------

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Bumped whenever the on-disk history shape changes. Stamped into
/// `PRAGMA user_version`, which starts life at 0 on every SQLite database —
/// so 0 doubles as "pre-versioning", exactly the databases that need
/// migrating.
const SCHEMA_VERSION: i64 = 1;

/// Version 1 stores each distinct value once in `value_blobs` instead of
/// repeating the plaintext in every `captured_values` row.
///
/// `value_blobs` is keyed by `(item, value_sha256)` rather than by the digest
/// alone. Global content-addressing would dedup marginally better, but it
/// lets two *different* names share one blob, and then `destroy --name` has no
/// correct move: keeping the blob leaves the name's plaintext readable, and
/// removing it erases another live name's history. Keying per item means a
/// blob is never shared across names, so destroy-by-name keeps the same
/// meaning it had when every row carried its own copy — and no reference
/// counting is required to get there.
const HISTORY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS entries (\
         sequence INTEGER PRIMARY KEY AUTOINCREMENT,\
         timestamp_ns INTEGER NOT NULL,\
         operation TEXT NOT NULL,\
         previous_hash TEXT NOT NULL,\
         entry_hash TEXT NOT NULL UNIQUE\
     ) STRICT;\
     CREATE TABLE IF NOT EXISTS value_blobs (\
         item TEXT NOT NULL,\
         value_sha256 TEXT NOT NULL,\
         value_blob BLOB NOT NULL,\
         PRIMARY KEY (item, value_sha256)\
     ) STRICT;\
     CREATE TABLE IF NOT EXISTS captured_values (\
         sequence INTEGER NOT NULL REFERENCES entries(sequence),\
         item TEXT NOT NULL,\
         value_sha256 TEXT NOT NULL,\
         destroyed_by INTEGER REFERENCES entries(sequence),\
         PRIMARY KEY (sequence, item)\
     ) STRICT;\
     CREATE TABLE IF NOT EXISTS head (\
         singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
         sequence INTEGER NOT NULL,\
         entry_hash TEXT NOT NULL\
     ) STRICT;";

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Brings an existing history database up to [`SCHEMA_VERSION`].
///
/// The schema above is created with `CREATE TABLE IF NOT EXISTS`, which is
/// silent about tables that already exist in an older shape — so without this
/// an upgraded binary would keep writing against a v0 layout and never say so.
fn migrate_history(conn: &Connection) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| operation_error(e.to_string()))?;

    if version >= SCHEMA_VERSION {
        return Ok(());
    }

    // A v0 database is only distinguishable from a freshly created one by
    // whether the legacy column is still there: both report user_version 0,
    // because the tables above were created before this function stamped
    // anything.
    if version == 0 && has_legacy_value_blob(conn)? {
        migrate_v0_to_v1(conn)?;
    }

    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        .map_err(|e| operation_error(e.to_string()))
}

fn has_legacy_value_blob(conn: &Connection) -> Result<bool> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info('captured_values')")
        .map_err(|e| operation_error(e.to_string()))?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| operation_error(e.to_string()))?;
    while let Some(row) = rows.next() {
        if row.map_err(|e| operation_error(e.to_string()))? == "value_blob" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Lifts per-row plaintext into `value_blobs` and drops the column.
///
/// Destroyed rows are deliberately skipped by the `value_blob IS NOT NULL`
/// filter: their bytes are already gone and must stay gone. They keep their
/// `value_sha256`, which is what makes a tombstone auditable.
fn migrate_v0_to_v1(conn: &Connection) -> Result<()> {
    // SQLite's documented recipe for rebuilding a table: foreign keys off for
    // the duration, since the rename below would otherwise be seen against a
    // half-built table. `foreign_keys` is a no-op inside a transaction, so it
    // has to be set on either side of one.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| operation_error(e.to_string()))?;

    let rebuild = || -> Result<()> {
        // Real newlines rather than `\` continuations: a continuation also
        // eats the next line's indentation, which silently welded
        // `captured_values` onto `WHERE` here and produced a syntax error a
        // long way from its cause.
        //
        // No `CREATE TABLE value_blobs` here: `connection()` executes
        // `HISTORY_SCHEMA` before it calls `migrate_history`, so the table
        // always exists by now. The `CREATE TABLE IF NOT EXISTS` that used to
        // sit here was therefore unreachable -- a second copy of the schema
        // that no test could catch drifting from the real one, which is exactly
        // how mutation testing found it (mutating its primary key changed
        // nothing). If a future SCHEMA_VERSION reshapes `value_blobs`, this
        // ordering needs revisiting: the migration would then be filling a
        // table `HISTORY_SCHEMA` had already created in the *newer* shape.
        conn.execute_batch(
            r#"
BEGIN IMMEDIATE;

INSERT OR IGNORE INTO value_blobs (item, value_sha256, value_blob)
    SELECT item, value_sha256, value_blob FROM captured_values
    WHERE value_blob IS NOT NULL;

CREATE TABLE captured_values_v1 (
    sequence     INTEGER NOT NULL REFERENCES entries(sequence),
    item         TEXT NOT NULL,
    value_sha256 TEXT NOT NULL,
    destroyed_by INTEGER REFERENCES entries(sequence),
    PRIMARY KEY (sequence, item)
) STRICT;

INSERT INTO captured_values_v1 (sequence, item, value_sha256, destroyed_by)
    SELECT sequence, item, value_sha256, destroyed_by FROM captured_values;

DROP TABLE captured_values;
ALTER TABLE captured_values_v1 RENAME TO captured_values;

COMMIT;
"#,
        )
        .map_err(|e| operation_error(format!("history migration failed: {e}")))
    };

    let mut outcome = rebuild().and_then(|()| {
        // Verifying inside the migration, not in a test only: a rebuild that
        // silently dropped a reference would leave the ledger unprovable.
        let violations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check('captured_values')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| operation_error(e.to_string()))?;
        if violations > 0 {
            return Err(operation_error(format!(
                "history migration left {violations} dangling entry reference(s); \
                 refusing to continue"
            )));
        }
        Ok(())
    });

    if outcome.is_err() {
        // `execute_batch` stops at the first failing statement, so a failure
        // anywhere after `BEGIN` leaves the transaction open on a connection
        // this provider is about to hand out.
        if let Err(rollback) = conn.execute_batch("ROLLBACK;") {
            outcome = outcome.and(Err(operation_error(format!(
                "history migration failed and could not be rolled back: {rollback}"
            ))));
        }
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| operation_error(e.to_string()))?;

    outcome
}

#[derive(Debug)]
struct CapturedValue {
    item: String,
    value_sha256: String,
}

#[derive(Debug)]
struct Entry {
    sequence: i64,
    timestamp_ns: i64,
    operation: String,
    values: Vec<CapturedValue>,
    previous_hash: String,
    entry_hash: String,
}

fn canonical_entry_json(entry: &Entry) -> String {
    let mut map = BTreeMap::new();
    map.insert("sequence", serde_json::Value::Number(entry.sequence.into()));
    map.insert(
        "timestamp_ns",
        serde_json::Value::Number(entry.timestamp_ns.into()),
    );
    map.insert(
        "operation",
        serde_json::Value::String(entry.operation.clone()),
    );
    let values: Vec<serde_json::Value> = entry
        .values
        .iter()
        .map(|v| {
            let mut pair = serde_json::Map::new();
            pair.insert("item".into(), serde_json::Value::String(v.item.clone()));
            pair.insert(
                "value_sha256".into(),
                serde_json::Value::String(v.value_sha256.clone()),
            );
            serde_json::Value::Object(pair)
        })
        .collect();
    map.insert("values", serde_json::Value::Array(values));
    map.insert(
        "previous_hash",
        serde_json::Value::String(entry.previous_hash.clone()),
    );

    serde_json::to_string(&map).unwrap()
}

fn compute_entry_hash(previous_hash: &str, entry: &Entry) -> String {
    let canonical = canonical_entry_json(entry);
    sha256_hex(format!("{previous_hash}\n{canonical}").as_bytes())
}

/// Appends one entry to the history chain, snapshotting every live secret.
///
/// Public so callers that hold their own connection to the same database can
/// advance the chain without restating how it is computed. `destroy` needs
/// exactly that: tombstones reference the entry that destroyed them, so a name
/// with no live value left to delete has nothing to point at, and the verb used
/// to refuse rather than attribute the tombstone to an unrelated entry at the
/// tip. Reusing this keeps hashing in one place, which was the reason for that
/// refusal in the first place.
pub fn capture_history(conn: &Connection, operation: &str) -> Result<()> {
    let mut count: i64 = 0;
    let mut previous_hash = ZERO_HASH.to_string();

    let head_row: Option<(i64, String)> = conn
        .query_row(
            "SELECT sequence, entry_hash FROM head WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| operation_error(e.to_string()))?;

    if let Some((seq, hash)) = head_row {
        count = seq;
        previous_hash = hash;
    }

    let sequence = count + 1;
    let timestamp_ns = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| operation_error(format!("clock error: {e}")))?;
        now.as_nanos() as i64
    };

    let mut stmt = conn
        .prepare("SELECT item, value FROM secrets ORDER BY item")
        .map_err(|e| operation_error(e.to_string()))?;

    let mut pairs = Vec::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| operation_error(e.to_string()))?;

    for row in rows {
        pairs.push(row.map_err(|e| operation_error(e.to_string()))?);
    }

    let values: Vec<CapturedValue> = pairs
        .iter()
        .map(|(item, value)| CapturedValue {
            item: item.clone(),
            value_sha256: sha256_hex(value.as_bytes()),
        })
        .collect();

    let mut entry = Entry {
        sequence,
        timestamp_ns,
        operation: operation.to_string(),
        values,
        previous_hash: previous_hash.clone(),
        entry_hash: String::new(),
    };
    entry.entry_hash = compute_entry_hash(&previous_hash, &entry);

    conn.execute(
        "INSERT INTO entries (sequence, timestamp_ns, operation, previous_hash, entry_hash) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            entry.sequence,
            entry.timestamp_ns,
            entry.operation,
            entry.previous_hash,
            entry.entry_hash,
        ],
    )
    .map_err(|e| operation_error(e.to_string()))?;

    for (val, (_, plain)) in entry.values.iter().zip(pairs.iter()) {
        // `OR IGNORE` is the dedup: the second and later captures of an
        // unchanged value find the blob already present and store only the
        // reference below. This is the whole point of the table — history
        // snapshots every secret on every operation, so without it the
        // plaintext count grows with entries x secrets.
        conn.execute(
            "INSERT OR IGNORE INTO value_blobs (item, value_sha256, value_blob) \
             VALUES (?1, ?2, ?3)",
            params![val.item, val.value_sha256, plain.as_bytes()],
        )
        .map_err(|e| operation_error(e.to_string()))?;

        conn.execute(
            "INSERT INTO captured_values (sequence, item, value_sha256, destroyed_by) \
             VALUES (?1, ?2, ?3, NULL)",
            params![entry.sequence, val.item, val.value_sha256],
        )
        .map_err(|e| operation_error(e.to_string()))?;
    }

    conn.execute(
        "INSERT INTO head (singleton, sequence, entry_hash) VALUES (1, ?1, ?2) \
         ON CONFLICT(singleton) DO UPDATE SET \
         sequence = excluded.sequence, entry_hash = excluded.entry_hash",
        params![entry.sequence, entry.entry_hash],
    )
    .map_err(|e| operation_error(e.to_string()))?;

    Ok(())
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
        SqliteProvider::new(SqliteConfig {
            path,
            history: false,
        })
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
            "sqlite://./secrets.db?invalid=on",
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
    fn delete_leaves_no_plaintext_in_the_database_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("secrets.db");
        let provider = provider(path.clone());
        // Distinctive enough that a hit in the raw file cannot be coincidence,
        // and long enough to occupy a cell rather than fit in a header.
        let plaintext = "PLAINTEXT-CANARY-eb4c1f9a".repeat(8);

        provider
            .set(
                convention("KEY"),
                &SecretString::new(plaintext.clone().into()),
            )
            .unwrap();

        // Anti-vacuity: prove the scan can find the value at all. Without this
        // the test passes for free if the value never reached the file, or if
        // the needle is simply wrong.
        assert!(
            file_contains(&path, plaintext.as_bytes()),
            "precondition failed: plaintext is not in the file even before delete, \
             so its later absence would prove nothing"
        );

        assert!(provider.delete(convention("KEY")).unwrap());

        assert!(
            !file_contains(&path, plaintext.as_bytes()),
            "deleted plaintext is still readable in {}: PRAGMA secure_delete is \
             not in effect on the provider's connections",
            path.display()
        );
    }

    fn file_contains(path: &Path, needle: &[u8]) -> bool {
        let bytes = std::fs::read(path).unwrap();
        bytes.windows(needle.len()).any(|window| window == needle)
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

    #[test]
    fn foreign_keys_are_enforced_on_every_connection_this_provider_hands_out() {
        // `foreign_keys` is per-connection and off by default, so without the
        // pragma the REFERENCES clauses on captured_values are documentation:
        // a tombstone could name a `destroyed_by` sequence no entry has, and
        // the store would accept it.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        let provider = SqliteProvider::new(SqliteConfig {
            path: db_path.clone(),
            history: true,
        });
        provider
            .set(convention("APP_SECRET"), &SecretString::new("v".into()))
            .unwrap();

        let conn = provider.connection().unwrap();
        let enabled: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(enabled, 1, "foreign key enforcement must be on");

        // And prove it actually bites, rather than merely being reported on.
        let orphan = conn.execute(
            "INSERT INTO captured_values (sequence, item, value_sha256) \
             VALUES (99999, 'x', 'd')",
            [],
        );
        // Asserting the *reason*, not just `is_err()`. This insert once named
        // a `value_blob` column; when that column moved to `value_blobs` the
        // statement still failed — with "no such column" — and the weaker
        // assertion kept passing while testing nothing.
        let error = orphan.expect_err("a captured value referencing no entry must be refused");
        assert!(
            matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ConstraintViolation)
            ),
            "must be refused by the foreign key, got: {error}"
        );
    }

    #[test]
    fn history_is_captured_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        let provider = SqliteProvider::new(SqliteConfig {
            path: db_path.clone(),
            history: true,
        });

        provider
            .set(
                convention("APP_SECRET"),
                &SecretString::new("initial".into()),
            )
            .unwrap();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let entries: i64 = conn
            .query_row("SELECT count(*) FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entries, 1);

        let captured: i64 = conn
            .query_row("SELECT count(*) FROM captured_values", [], |row| row.get(0))
            .unwrap();
        assert_eq!(captured, 1);

        provider.delete(convention("APP_SECRET")).unwrap();

        let entries_after_delete: i64 = conn
            .query_row("SELECT count(*) FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entries_after_delete, 2);
    }

    /// The v0 schema, verbatim, so the migration is exercised against the
    /// shape that is actually on disk rather than a paraphrase of it.
    const LEGACY_SCHEMA_V0: &str = "CREATE TABLE entries (\
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,\
             timestamp_ns INTEGER NOT NULL,\
             operation TEXT NOT NULL,\
             previous_hash TEXT NOT NULL,\
             entry_hash TEXT NOT NULL UNIQUE\
         ) STRICT;\
         CREATE TABLE captured_values (\
             sequence INTEGER NOT NULL REFERENCES entries(sequence),\
             item TEXT NOT NULL,\
             value_blob BLOB,\
             value_sha256 TEXT NOT NULL,\
             destroyed_by INTEGER REFERENCES entries(sequence),\
             PRIMARY KEY (sequence, item),\
             CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))\
         ) STRICT;\
         CREATE TABLE head (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             sequence INTEGER NOT NULL,\
             entry_hash TEXT NOT NULL\
         ) STRICT;";

    /// Builds a v0 database holding the interesting cases at once: one value
    /// repeated across snapshots (the dedup win), two *different* names
    /// sharing one value (the reason blobs are keyed per item), and a
    /// destroyed row whose bytes are already gone.
    fn legacy_v0_database(path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(LEGACY_SCHEMA_V0).unwrap();
        for seq in 1..=3 {
            conn.execute(
                "INSERT INTO entries (sequence, timestamp_ns, operation, previous_hash, entry_hash) \
                 VALUES (?1, ?2, 'source-set', 'prev', ?3)",
                params![seq, seq * 1000, format!("hash{seq}")],
            )
            .unwrap();
        }

        let repeated = b"same-every-time";
        let repeated_digest = sha256_hex(repeated);
        for seq in 1..=3 {
            conn.execute(
                "INSERT INTO captured_values (sequence, item, value_blob, value_sha256, destroyed_by) \
                 VALUES (?1, 'p/prod/STABLE', ?2, ?3, NULL)",
                params![seq, repeated, repeated_digest],
            )
            .unwrap();
        }

        // Two names, one value — global content-addressing would collapse
        // these into a single blob and make destroy-by-name unanswerable.
        let shared = b"shared-between-two-names";
        let shared_digest = sha256_hex(shared);
        for item in ["p/prod/ALPHA", "p/prod/BETA"] {
            conn.execute(
                "INSERT INTO captured_values (sequence, item, value_blob, value_sha256, destroyed_by) \
                 VALUES (1, ?1, ?2, ?3, NULL)",
                params![item, shared, shared_digest],
            )
            .unwrap();
        }

        // Already destroyed under v0: blob NULL, digest retained.
        conn.execute(
            "INSERT INTO captured_values (sequence, item, value_blob, value_sha256, destroyed_by) \
             VALUES (2, 'p/prod/GONE', NULL, 'deadbeef', 3)",
            [],
        )
        .unwrap();

        conn
    }

    fn open_migrated(path: &std::path::Path) -> rusqlite::Connection {
        let provider = SqliteProvider::new(SqliteConfig {
            path: path.to_path_buf(),
            history: true,
        });
        provider.connection().unwrap()
    }

    #[test]
    fn migration_dedups_blobs_without_resurrecting_destroyed_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        drop(legacy_v0_database(&db_path));

        let conn = open_migrated(&db_path);

        // Every row survives — the migration moves bytes, it does not drop
        // history.
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM captured_values", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 6, "all captured rows must survive the migration");

        // 3 snapshots of one value collapse to 1 blob; the shared value is
        // stored once per name, not once globally; the destroyed row
        // contributes nothing.
        let blobs: i64 = conn
            .query_row("SELECT count(*) FROM value_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(blobs, 3, "expected STABLE + ALPHA + BETA, deduped");

        let destroyed_blob: i64 = conn
            .query_row(
                "SELECT count(*) FROM value_blobs WHERE item = 'p/prod/GONE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            destroyed_blob, 0,
            "destroyed bytes must not come back from the dead"
        );

        // The tombstone itself is untouched: digest retained, destroyer named.
        let (digest, destroyed_by): (String, i64) = conn
            .query_row(
                "SELECT value_sha256, destroyed_by FROM captured_values WHERE item = 'p/prod/GONE'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(digest, "deadbeef");
        assert_eq!(destroyed_by, 3);

        // The legacy column is gone and the version is stamped.
        assert!(!has_legacy_value_blob(&conn).unwrap());
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migration_keeps_identical_values_separate_per_name() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        drop(legacy_v0_database(&db_path));

        let conn = open_migrated(&db_path);

        // Same bytes, same digest, two names: two blob rows. Collapsing these
        // is what would leave `destroy ALPHA` unable to remove ALPHA's
        // plaintext without also erasing BETA's live history.
        let shared: i64 = conn
            .query_row(
                "SELECT count(*) FROM value_blobs WHERE item IN ('p/prod/ALPHA', 'p/prod/BETA')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(shared, 2, "a blob must never be shared across names");

        let distinct_digests: i64 = conn
            .query_row(
                "SELECT count(DISTINCT value_sha256) FROM value_blobs \
                 WHERE item IN ('p/prod/ALPHA', 'p/prod/BETA')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(distinct_digests, 1, "and they are genuinely the same value");
    }

    #[test]
    fn migration_is_idempotent_and_preserves_readability() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        drop(legacy_v0_database(&db_path));

        drop(open_migrated(&db_path));
        let conn = open_migrated(&db_path);

        let blobs: i64 = conn
            .query_row("SELECT count(*) FROM value_blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(blobs, 3, "a second open must not re-run the migration");

        // The bytes are still reachable by the join the restore path uses.
        let value: Vec<u8> = conn
            .query_row(
                "SELECT b.value_blob FROM captured_values cv \
                 JOIN value_blobs b ON b.item = cv.item AND b.value_sha256 = cv.value_sha256 \
                 WHERE cv.sequence = 1 AND cv.item = 'p/prod/STABLE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, b"same-every-time");
    }

    #[test]
    fn a_fresh_database_is_created_already_at_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        let provider = SqliteProvider::new(SqliteConfig {
            path: db_path.clone(),
            history: true,
        });
        provider
            .set(convention("APP_SECRET"), &SecretString::new("v".into()))
            .unwrap();

        let conn = provider.connection().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(!has_legacy_value_blob(&conn).unwrap());
    }

    #[test]
    fn repeated_captures_of_one_value_store_its_bytes_once() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");
        let provider = SqliteProvider::new(SqliteConfig {
            path: db_path.clone(),
            history: true,
        });

        // Setting a second, unrelated secret re-snapshots the first one, which
        // is exactly how history grows with entries x secrets.
        provider
            .set(convention("APP_SECRET"), &SecretString::new("held".into()))
            .unwrap();
        for i in 0..4 {
            provider
                .set(
                    convention(&format!("OTHER_{i}")),
                    &SecretString::new("x".into()),
                )
                .unwrap();
        }

        let conn = provider.connection().unwrap();
        let captured: i64 = conn
            .query_row(
                "SELECT count(*) FROM captured_values WHERE item = ?1",
                params!["project/production/APP_SECRET"],
                |row| row.get(0),
            )
            .unwrap();
        let blobs: i64 = conn
            .query_row(
                "SELECT count(*) FROM value_blobs WHERE item = ?1",
                params!["project/production/APP_SECRET"],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            captured > 1,
            "the value should be captured in several snapshots, got {captured}"
        );
        assert_eq!(blobs, 1, "but its bytes should be stored exactly once");
    }
}
