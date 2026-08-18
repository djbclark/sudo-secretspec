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
            init_sql.push_str(&history_schema());
        }
        conn.execute_batch(&init_sql).map_err(|error| {
            operation_error(format!(
                "failed to prepare sqlite database '{}': {error}",
                self.config.path.display()
            ))
        })?;
        if self.config.history {
            migrate_history(&conn)?;
            conn.execute_batch(HISTORY_POST_MIGRATION)
                .map_err(|error| {
                    operation_error(format!(
                        "failed to finish history schema on '{}': {error}",
                        self.config.path.display()
                    ))
                })?;
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
const SCHEMA_VERSION: i64 = 2;

/// Each distinct value is stored once in `value_blobs` instead of repeating
/// the plaintext in every `captured_values` row.
///
/// `value_blobs` is keyed by `(item, value_sha256)` rather than by the digest
/// alone. Global content-addressing would dedup marginally better, but it
/// lets two *different* names share one blob, and then `destroy --name` has no
/// correct move: keeping the blob leaves the name's plaintext readable, and
/// removing it erases another live name's history. Keying per item means a
/// blob is never shared across names, so destroy-by-name keeps the same
/// meaning it had when every row carried its own copy — and no reference
/// counting is required to get there.
///
/// Version 2 adds a surrogate `blob_id` and makes the liveness invariant
/// structural rather than something every query has to remember.
///
/// `captured_values.blob_id` records which blob a snapshot captured and is
/// never rewritten. `live_blob_id` is generated from it and nulls itself the
/// moment `destroyed_by` is set, and it — not the raw column — carries the
/// foreign key. Three properties fall out of that arrangement:
///
/// * `destroy` stays a single-column write to `destroyed_by`. The reference
///   releases itself, so no query can leave the two out of step.
/// * `ON DELETE RESTRICT` refuses to drop a blob a *live* snapshot still
///   references, which replaces the hand-written `NOT IN (…)` guard the
///   broker used to carry.
/// * A tombstone cannot be resurrected. Clearing `destroyed_by` re-exposes
///   the old `blob_id`, whose blob is gone, and the foreign key refuses the
///   update. Under a digest-keyed reference this was the one hole a `CHECK`
///   could not close: destroy a name, re-set it to the same value, and the
///   recreated `(item, value_sha256)` row made the tombstone restorable
///   again.
///
/// That last property depends on `AUTOINCREMENT`, which is load-bearing here
/// and must not be removed as the tuning advice in SQLite's own documentation
/// suggests. It is what guarantees a `blob_id` is never handed out twice. With
/// a plain `INTEGER PRIMARY KEY` the recreated blob reclaims the freed rowid,
/// the dangling reference becomes valid again, and the tombstone resurrects.
///
/// `value_sha256` stays on `captured_values` even though `value_blobs` also
/// holds it. It is what makes a tombstone auditable once its bytes are gone,
/// and it is hashed into the entry chain, so removing it would invalidate
/// every existing `entry_hash`.
/// Column and constraint body of `value_blobs`, without the `CREATE TABLE`
/// wrapper, so the migration that rebuilds the table cannot drift from the
/// definition the provider creates.
const VALUE_BLOBS_BODY: &str = "\
     blob_id INTEGER PRIMARY KEY AUTOINCREMENT,\
     item TEXT NOT NULL,\
     value_sha256 TEXT NOT NULL,\
     value_blob BLOB NOT NULL,\
     UNIQUE (item, value_sha256)";

/// Column and constraint body of `captured_values`. Shared with the migration
/// for the same reason as [`VALUE_BLOBS_BODY`] — an earlier version of this
/// file kept a second copy of the schema inside the migration, and mutation
/// testing found it precisely because nothing could observe the two drifting.
const CAPTURED_VALUES_BODY: &str = "\
     sequence INTEGER NOT NULL REFERENCES entries(sequence),\
     item TEXT NOT NULL,\
     value_sha256 TEXT NOT NULL,\
     blob_id INTEGER,\
     destroyed_by INTEGER REFERENCES entries(sequence),\
     live_blob_id INTEGER GENERATED ALWAYS AS \
         (CASE WHEN destroyed_by IS NULL THEN blob_id END) VIRTUAL \
         REFERENCES value_blobs(blob_id) ON DELETE RESTRICT,\
     PRIMARY KEY (sequence, item),\
     CHECK (blob_id IS NOT NULL OR destroyed_by IS NOT NULL),\
     CHECK (destroyed_by IS NULL OR destroyed_by > sequence)";

fn history_schema() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS entries (\
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,\
             timestamp_ns INTEGER NOT NULL,\
             operation TEXT NOT NULL,\
             previous_hash TEXT NOT NULL,\
             entry_hash TEXT NOT NULL UNIQUE\
         ) STRICT;\
         CREATE TABLE IF NOT EXISTS value_blobs ({VALUE_BLOBS_BODY}) STRICT;\
         CREATE TABLE IF NOT EXISTS captured_values ({CAPTURED_VALUES_BODY}) STRICT;\
         CREATE TABLE IF NOT EXISTS head (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             sequence INTEGER NOT NULL,\
             entry_hash TEXT NOT NULL\
         ) STRICT;"
    )
}

/// Everything that can only be built once `captured_values` is known to be in
/// its current shape. Applied after [`migrate_history`], never as part of
/// [`history_schema`].
///
/// The index is here rather than in the schema because the schema runs first,
/// against a table `CREATE TABLE IF NOT EXISTS` silently leaves in whatever
/// shape it already had — so on an unmigrated database, indexing `blob_id`
/// fails with `no such column`. This ordering is exactly what the version
/// branch in `migrate_history` exists to respect.
///
/// `CREATE TRIGGER IF NOT EXISTS` matches on name alone, so it would keep an
/// old body forever once one existed — a stale invariant that still looks
/// present. Dropping first is what makes the definition here authoritative.
///
/// The triggers must also come after migration, because a migration rebuilds
/// `captured_values` and a no-`DELETE` trigger on the old table would block
/// copying rows out of it. `DROP TABLE` does not fire `BEFORE DELETE`
/// triggers, so the rebuild itself stays legal either way.
const HISTORY_POST_MIGRATION: &str = "\
     CREATE INDEX IF NOT EXISTS captured_values_blob_id \
         ON captured_values(item, blob_id);\
     DROP TRIGGER IF EXISTS captured_values_destroyed_by_is_write_once;\
     CREATE TRIGGER captured_values_destroyed_by_is_write_once \
         BEFORE UPDATE OF destroyed_by ON captured_values \
         FOR EACH ROW WHEN OLD.destroyed_by IS NOT NULL \
     BEGIN \
         SELECT RAISE(ABORT, 'captured_values.destroyed_by is write-once'); \
     END;\
     DROP TRIGGER IF EXISTS captured_values_is_append_only;\
     CREATE TRIGGER captured_values_is_append_only \
         BEFORE DELETE ON captured_values \
         FOR EACH ROW \
     BEGIN \
         SELECT RAISE(ABORT, 'captured_values is append-only'); \
     END;";

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
    // anything. A fresh database needs no migration — `history_schema()`
    // already built it in the current shape — so it falls through to the
    // stamp alone.
    if version == 0 && has_legacy_value_blob(conn)? {
        migrate_v0_to_v2(conn)?;
    } else if version == 1 {
        migrate_v1_to_v2(conn)?;
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

/// Rows a migration refuses to interpret, as `(sequence, item)` pairs.
///
/// Capped, because the point is to name the problem rather than to print an
/// entire corrupt table into an error message.
fn offending_rows(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| operation_error(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(format!(
                "(sequence {}, item {})",
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?
            ))
        })
        .map_err(|e| operation_error(e.to_string()))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| operation_error(e.to_string()))?);
        if out.len() == 20 {
            break;
        }
    }
    Ok(out)
}

/// Runs a table rebuild with the pragma handling and verification every
/// migration needs, so each one only has to state its own SQL.
///
/// SQLite's documented recipe for rebuilding a table wants foreign keys off
/// for the duration, since the renames would otherwise be seen against a
/// half-built table. `foreign_keys` is a no-op inside a transaction, so it has
/// to be set on either side of one.
fn rebuild_history(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| operation_error(e.to_string()))?;

    let mut outcome = conn
        .execute_batch(sql)
        .map_err(|e| operation_error(format!("history migration failed: {e}")))
        .and_then(|()| {
            // Verifying inside the migration, not in a test only: a rebuild
            // that silently dropped a reference would leave the ledger
            // unprovable. `foreign_key_check` now also covers the generated
            // `live_blob_id`, so this proves every live snapshot still points
            // at bytes that exist.
            let violations: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_foreign_key_check('captured_values')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| operation_error(e.to_string()))?;
            if violations > 0 {
                return Err(operation_error(format!(
                    "history migration left {violations} dangling reference(s); \
                     refusing to continue"
                )));
            }
            let integrity: String = conn
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .map_err(|e| operation_error(e.to_string()))?;
            if integrity != "ok" {
                return Err(operation_error(format!(
                    "history migration left the database inconsistent: {integrity}"
                )));
            }
            Ok(())
        });

    if outcome.is_err() {
        // `execute_batch` stops at the first failing statement, so a failure
        // anywhere after `BEGIN` leaves the transaction open on a connection
        // this provider is about to hand out.
        if let Err(rollback) = conn.execute_batch("ROLLBACK;")
            && !rollback.to_string().contains("no transaction is active")
        {
            outcome = outcome.and(Err(operation_error(format!(
                "history migration failed and could not be rolled back: {rollback}"
            ))));
        }
    }

    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| operation_error(e.to_string()))?;

    outcome
}

/// Lifts per-row plaintext into `value_blobs` and rebuilds `captured_values`
/// around a surrogate `blob_id`.
///
/// Destroyed rows are deliberately skipped by the `value_blob IS NOT NULL`
/// filter: their bytes are already gone and must stay gone. They keep their
/// `value_sha256`, which is what makes a tombstone auditable, and they are
/// given a NULL `blob_id` even when a live row happens to hold the same value
/// under the same name. Pointing them at that live blob would be technically
/// consistent and would quietly recreate the resurrection hole this version
/// exists to close.
///
/// `value_blobs` is not created here. `connection()` executes the schema
/// before it calls `migrate_history`, so on a v0 database the table already
/// exists — empty, and in the current shape, because v0 never had one.
fn migrate_v0_to_v2(conn: &Connection) -> Result<()> {
    // Fail closed. A destroyed row that kept its bytes means `destroy` did not
    // do what the ledger says it did; a live row with no bytes means one was
    // lost. Both corrupt the exact property this history exists to prove, and
    // neither has a repair that is obviously right, so refuse and name them.
    let bad = offending_rows(
        conn,
        "SELECT sequence, item FROM captured_values \
         WHERE (destroyed_by IS NULL) = (value_blob IS NULL) \
         ORDER BY sequence, item",
    )?;
    if !bad.is_empty() {
        return Err(operation_error(format!(
            "history migration refused: {} row(s) where the tombstone and the stored \
             bytes disagree — a destroyed row that kept its plaintext, or a live row \
             with none: {}",
            bad.len(),
            bad.join(", ")
        )));
    }

    // Real newlines rather than `\` continuations: a continuation also eats the
    // next line's indentation, which silently welded `captured_values` onto
    // `WHERE` here and produced a syntax error a long way from its cause.
    rebuild_history(
        conn,
        &format!(
            r#"
BEGIN IMMEDIATE;

INSERT OR IGNORE INTO value_blobs (item, value_sha256, value_blob)
    SELECT item, value_sha256, value_blob FROM captured_values
    WHERE value_blob IS NOT NULL;

CREATE TABLE captured_values_new ({CAPTURED_VALUES_BODY}) STRICT;

INSERT INTO captured_values_new
        (sequence, item, value_sha256, blob_id, destroyed_by)
    SELECT cv.sequence, cv.item, cv.value_sha256,
           CASE WHEN cv.destroyed_by IS NULL
                THEN (SELECT b.blob_id FROM value_blobs b
                       WHERE b.item = cv.item
                         AND b.value_sha256 = cv.value_sha256)
           END,
           cv.destroyed_by
      FROM captured_values cv;

DROP TABLE captured_values;
ALTER TABLE captured_values_new RENAME TO captured_values;

COMMIT;
"#
        ),
    )
}

/// Rebuilds both history tables around `blob_id`.
///
/// v1 addressed a blob by `(item, value_sha256)` from `captured_values`, which
/// made a tombstone restorable again as soon as the same value was re-set
/// under the same name — the recreated row matched the old digest. Identity
/// replaces content here, and `AUTOINCREMENT` guarantees the identity is never
/// reissued.
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    let bad = offending_rows(
        conn,
        "SELECT cv.sequence, cv.item FROM captured_values cv \
         WHERE cv.destroyed_by IS NULL \
           AND NOT EXISTS (SELECT 1 FROM value_blobs b \
                            WHERE b.item = cv.item \
                              AND b.value_sha256 = cv.value_sha256) \
         ORDER BY cv.sequence, cv.item",
    )?;
    if !bad.is_empty() {
        return Err(operation_error(format!(
            "history migration refused: {} live row(s) whose captured bytes are \
             missing from value_blobs: {}",
            bad.len(),
            bad.join(", ")
        )));
    }

    rebuild_history(
        conn,
        &format!(
            r#"
BEGIN IMMEDIATE;

CREATE TABLE value_blobs_new ({VALUE_BLOBS_BODY}) STRICT;

INSERT INTO value_blobs_new (item, value_sha256, value_blob)
    SELECT item, value_sha256, value_blob FROM value_blobs;

CREATE TABLE captured_values_new ({CAPTURED_VALUES_BODY}) STRICT;

INSERT INTO captured_values_new
        (sequence, item, value_sha256, blob_id, destroyed_by)
    SELECT cv.sequence, cv.item, cv.value_sha256,
           CASE WHEN cv.destroyed_by IS NULL
                THEN (SELECT b.blob_id FROM value_blobs_new b
                       WHERE b.item = cv.item
                         AND b.value_sha256 = cv.value_sha256)
           END,
           cv.destroyed_by
      FROM captured_values cv;

DROP TABLE captured_values;
DROP TABLE value_blobs;
ALTER TABLE value_blobs_new RENAME TO value_blobs;
ALTER TABLE captured_values_new RENAME TO captured_values;

COMMIT;
"#
        ),
    )
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

        // Read the id back rather than using `last_insert_rowid()`: on the
        // dedup path above the insert is ignored, and the id that matters is
        // the existing blob's, not whatever this connection inserted last.
        let blob_id: i64 = conn
            .query_row(
                "SELECT blob_id FROM value_blobs WHERE item = ?1 AND value_sha256 = ?2",
                params![val.item, val.value_sha256],
                |row| row.get(0),
            )
            .map_err(|e| operation_error(e.to_string()))?;

        conn.execute(
            "INSERT INTO captured_values \
                 (sequence, item, value_sha256, blob_id, destroyed_by) \
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![entry.sequence, val.item, val.value_sha256, blob_id],
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

        // Give the row a real blob, so the only thing wrong with it is the
        // entry reference. Without this the `blob_id IS NOT NULL OR
        // destroyed_by IS NOT NULL` check fires first and the assertion below
        // passes without the foreign key ever being consulted.
        conn.execute(
            "INSERT INTO value_blobs (item, value_sha256, value_blob) VALUES ('x', 'd', x'00')",
            [],
        )
        .unwrap();
        let blob_id: i64 = conn
            .query_row(
                "SELECT blob_id FROM value_blobs WHERE item = 'x' AND value_sha256 = 'd'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // And prove it actually bites, rather than merely being reported on.
        let orphan = conn.execute(
            "INSERT INTO captured_values (sequence, item, value_sha256, blob_id) \
             VALUES (99999, 'x', 'd', ?1)",
            params![blob_id],
        );
        // Asserting the *reason*, not just `is_err()`. This insert once named
        // a `value_blob` column; when that column moved to `value_blobs` the
        // statement still failed — with "no such column" — and the weaker
        // assertion kept passing while testing nothing. Naming the foreign key
        // specifically is the same defence against the CHECK constraints that
        // now sit on this table.
        let error = orphan.expect_err("a captured value referencing no entry must be refused");
        assert!(
            matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ConstraintViolation)
            ),
            "must be refused by a constraint, got: {error}"
        );
        assert!(
            error.to_string().contains("FOREIGN KEY"),
            "must be refused by the foreign key specifically, got: {error}"
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

    /// The v1 schema, verbatim. v1 shipped in no release, but any working
    /// tree that ran the previous revision has databases in this shape, and a
    /// migration that only understood v0 would leave them permanently wrong
    /// while reporting nothing.
    const LEGACY_SCHEMA_V1: &str = "CREATE TABLE entries (\
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,\
             timestamp_ns INTEGER NOT NULL,\
             operation TEXT NOT NULL,\
             previous_hash TEXT NOT NULL,\
             entry_hash TEXT NOT NULL UNIQUE\
         ) STRICT;\
         CREATE TABLE value_blobs (\
             item TEXT NOT NULL,\
             value_sha256 TEXT NOT NULL,\
             value_blob BLOB NOT NULL,\
             PRIMARY KEY (item, value_sha256)\
         ) STRICT;\
         CREATE TABLE captured_values (\
             sequence INTEGER NOT NULL REFERENCES entries(sequence),\
             item TEXT NOT NULL,\
             value_sha256 TEXT NOT NULL,\
             destroyed_by INTEGER REFERENCES entries(sequence),\
             PRIMARY KEY (sequence, item)\
         ) STRICT;\
         CREATE TABLE head (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             sequence INTEGER NOT NULL,\
             entry_hash TEXT NOT NULL\
         ) STRICT;\
         PRAGMA user_version = 1;";

    #[test]
    fn a_v1_database_migrates_and_stops_matching_tombstones_by_digest() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(LEGACY_SCHEMA_V1).unwrap();
            for seq in 1..=3 {
                conn.execute(
                    "INSERT INTO entries (sequence, timestamp_ns, operation, previous_hash, entry_hash) \
                     VALUES (?1, ?2, 'source-set', 'prev', ?3)",
                    params![seq, seq * 1000, format!("hash{seq}")],
                )
                .unwrap();
            }
            let value = b"recurring";
            let digest = sha256_hex(value);
            conn.execute(
                "INSERT INTO value_blobs (item, value_sha256, value_blob) VALUES ('p/prod/A', ?1, ?2)",
                params![digest, value],
            )
            .unwrap();
            // Sequence 1 was destroyed; sequence 3 later captured the same
            // value again under the same name, which is what recreated the
            // blob. Under v1 those two rows are indistinguishable to a join on
            // `(item, value_sha256)` — the bug this version removes.
            conn.execute(
                "INSERT INTO captured_values (sequence, item, value_sha256, destroyed_by) \
                 VALUES (1, 'p/prod/A', ?1, 2)",
                params![digest],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO captured_values (sequence, item, value_sha256, destroyed_by) \
                 VALUES (3, 'p/prod/A', ?1, NULL)",
                params![digest],
            )
            .unwrap();
        }

        let conn = open_migrated(&db_path);

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // Anti-vacuity: both rows still carry the same digest, so a digest
        // match would still conflate them. The test below has to be excluding
        // the tombstone some other way.
        let same_digest: i64 = conn
            .query_row(
                "SELECT count(DISTINCT value_sha256) FROM captured_values WHERE item = 'p/prod/A'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(same_digest, 1, "precondition: both rows share one digest");

        // The live row resolves to bytes; the tombstone resolves to nothing.
        let live: i64 = conn
            .query_row(
                "SELECT count(*) FROM captured_values cv \
                 JOIN value_blobs b ON b.blob_id = cv.live_blob_id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            live, 1,
            "only the undestroyed snapshot may resolve to bytes"
        );

        let tombstone_blob: Option<i64> = conn
            .query_row(
                "SELECT blob_id FROM captured_values WHERE sequence = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            tombstone_blob, None,
            "a migrated tombstone must not be given the recreated blob"
        );
    }

    #[test]
    fn a_migration_refuses_a_database_whose_tombstones_contradict_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("secrets.db");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            // The v0 CHECK constraint encodes the very invariant being
            // violated here, so the row has to be inserted without it.
            conn.execute_batch(&LEGACY_SCHEMA_V0.replace(
                "CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))",
                "CHECK (1)",
            ))
            .unwrap();
            conn.execute(
                "INSERT INTO entries (sequence, timestamp_ns, operation, previous_hash, entry_hash) \
                 VALUES (1, 1, 'source-set', 'prev', 'h1')",
                [],
            )
            .unwrap();
            // Destroyed, yet still holding its plaintext: `destroy` did not do
            // what the ledger says it did.
            conn.execute(
                "INSERT INTO captured_values (sequence, item, value_blob, value_sha256, destroyed_by) \
                 VALUES (1, 'p/prod/LIAR', x'0102', 'digest', 1)",
                [],
            )
            .unwrap();
        }

        let provider = SqliteProvider::new(SqliteConfig {
            path: db_path.clone(),
            history: true,
        });
        let error = provider
            .connection()
            .expect_err("a contradictory database must not be migrated");
        let message = error.to_string();
        assert!(
            message.contains("refused") && message.contains("p/prod/LIAR"),
            "the refusal must name the offending row, got: {message}"
        );

        // And it must refuse rather than half-apply: the database is still v0.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0, "a refused migration must not stamp a version");
    }

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

    /// A provider with history on, holding one captured value, plus the id of
    /// the blob that value lives in.
    fn history_with_one_capture(path: &std::path::Path) -> (SqliteProvider, Connection, i64) {
        let provider = SqliteProvider::new(SqliteConfig {
            path: path.to_path_buf(),
            history: true,
        });
        provider
            .set(convention("KEY"), &SecretString::new("v".into()))
            .unwrap();
        let conn = provider.connection().unwrap();
        let blob_id: i64 = conn
            .query_row("SELECT blob_id FROM value_blobs", [], |row| row.get(0))
            .unwrap();
        (provider, conn, blob_id)
    }

    #[test]
    fn a_blob_a_live_snapshot_still_references_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (_provider, conn, blob_id) = history_with_one_capture(&dir.path().join("secrets.db"));

        let error = conn
            .execute(
                "DELETE FROM value_blobs WHERE blob_id = ?1",
                params![blob_id],
            )
            .expect_err("a blob a live snapshot references must not be deletable");
        assert!(
            error.to_string().contains("FOREIGN KEY"),
            "must be refused by ON DELETE RESTRICT, got: {error}"
        );
    }

    #[test]
    fn a_tombstone_cannot_be_resurrected_even_when_its_value_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.db");
        let (provider, conn, blob_id) = history_with_one_capture(&path);

        // Tombstone the snapshot the way `destroy` does — one column — and
        // drop the bytes it released.
        conn.execute(
            "INSERT INTO entries (timestamp_ns, operation, previous_hash, entry_hash) \
             VALUES (0, 'destroy', 'p', 'h')",
            [],
        )
        .unwrap();
        let destroyer: i64 = conn.last_insert_rowid();
        // `capture_history` takes the next sequence from `head`, so an entry
        // written by hand has to advance it too or the `set` below collides.
        conn.execute(
            "UPDATE head SET sequence = ?1, entry_hash = 'h' WHERE singleton = 1",
            params![destroyer],
        )
        .unwrap();
        conn.execute(
            "UPDATE captured_values SET destroyed_by = ?1 WHERE sequence = 1",
            params![destroyer],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM value_blobs WHERE blob_id = ?1",
            params![blob_id],
        )
        .expect("once tombstoned, the blob releases and can be removed");

        // The same value comes back under the same name: byte-identical, same
        // digest, same item — the case that used to make the tombstone
        // restorable again.
        provider
            .set(convention("KEY"), &SecretString::new("v".into()))
            .unwrap();
        let recreated: i64 = conn
            .query_row(
                "SELECT blob_id FROM value_blobs ORDER BY blob_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(
            recreated, blob_id,
            "precondition: AUTOINCREMENT must not reissue the freed id, or the \
             dangling reference below would become valid again and this test \
             would prove nothing"
        );

        let error = conn
            .execute(
                "UPDATE captured_values SET destroyed_by = NULL WHERE sequence = 1",
                [],
            )
            .expect_err("a tombstone must not be clearable");
        // The trigger is deliberately not the only guard here: with it removed
        // the foreign key still refuses, because the row's `blob_id` names a
        // blob that no longer exists.
        assert!(
            error.to_string().contains("write-once") || error.to_string().contains("FOREIGN KEY"),
            "must be refused by the trigger or the foreign key, got: {error}"
        );
    }

    #[test]
    fn captured_values_rows_cannot_be_deleted_or_their_tombstones_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let (_provider, conn, _) = history_with_one_capture(&dir.path().join("secrets.db"));

        let deleted = conn
            .execute("DELETE FROM captured_values WHERE sequence = 1", [])
            .expect_err("history rows must not be deletable");
        assert!(
            deleted.to_string().contains("append-only"),
            "must be refused by the append-only trigger, got: {deleted}"
        );

        conn.execute(
            "INSERT INTO entries (timestamp_ns, operation, previous_hash, entry_hash) \
             VALUES (0, 'destroy', 'p', 'h')",
            [],
        )
        .unwrap();
        let first: i64 = conn.last_insert_rowid();
        conn.execute(
            "UPDATE captured_values SET destroyed_by = ?1 WHERE sequence = 1",
            params![first],
        )
        .expect("the first tombstone must be allowed");

        // Re-attributing an existing tombstone to a different entry would let
        // the ledger disagree with itself about what destroyed a value.
        let rewritten = conn
            .execute(
                "UPDATE captured_values SET destroyed_by = ?1 WHERE sequence = 1",
                params![first],
            )
            .expect_err("destroyed_by must not be rewritable");
        assert!(
            rewritten.to_string().contains("write-once"),
            "must be refused by the write-once trigger, got: {rewritten}"
        );
    }

    #[test]
    fn a_tombstone_may_not_name_an_entry_at_or_before_its_own_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let (_provider, conn, _) = history_with_one_capture(&dir.path().join("secrets.db"));

        let error = conn
            .execute(
                "UPDATE captured_values SET destroyed_by = 1 WHERE sequence = 1",
                [],
            )
            .expect_err("a value cannot be destroyed by the entry that captured it");
        assert!(
            error.to_string().contains("CHECK"),
            "must be refused by the ordering check, got: {error}"
        );
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
