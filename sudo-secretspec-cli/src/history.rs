//! Infinite secret history for the sudo-secretspec privilege boundary.
//!
//! The boundary already snapshots the manifest and the dotenv before every
//! mutating operation — see [`crate::broker`]'s `Mutation`, which copies both
//! files and, on success, deletes the copies. This module is where those bytes
//! go instead, so that a logical loss is a `restore` rather than an off-machine
//! backup recovery.
//!
//! ## Shape
//!
//! Two stores, because the two files need different things:
//!
//! - **The manifest** is kept whole. Its formatting, comment placement and
//!   ordering all carry meaning, and it holds *declarations, not values*, so
//!   nothing in it ever has to be destroyed.
//! - **The dotenv** is decomposed into one row per secret name. That is what
//!   makes destroy-by-name possible: removing one name's value from a
//!   whole-file blob would change that blob's digest and break the chain, and
//!   nulling the whole blob would take every other name's history with it.
//!
//! Exact reassembly of the dotenv is guaranteed by storing the digest of the
//! original file and re-rendering through the same `dotenv-ng` grammar the
//! engine's provider writes with. [`capture`] checks that round trip *before*
//! storing anything, so a file this module cannot reproduce is refused at
//! capture rather than discovered at restore.
//!
//! ## Chain
//!
//! Entries are hash-chained exactly as [`crate::audit`] chains events, and the
//! database is opened through the ledger's own hardening via
//! [`crate::audit::open_protected_db`] rather than a second copy of it.
//!
//! The chain covers each value's **digest**, never its bytes. That is what
//! makes destruction auditable: a destroyed row still proves that bytes with
//! digest X were captured at that point and were destroyed by entry N, without
//! retaining the value. `destroyed_by` is deliberately outside the hash — a
//! destroy must not invalidate the chain — and cannot be forged usefully
//! because the schema requires a row to hold its blob if and only if it is not
//! destroyed.
//!
//! ## Residual risk
//!
//! Identical to the ledger's, and for the same arithmetic reason: the chain
//! detects modification of any entry but the last, and detects nothing about
//! truncation or deletion of the whole store by a principal who can already
//! write the vault as root or the service user. See [`crate::audit`]'s module
//! documentation.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::audit::{self, AuditError, VerifyMode, ZERO_HASH, sha256_hex};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The fixed history filename inside the protected directory.
pub const DB_NAME: &str = "broker-history.sqlite3";

/// Upper bound on a single captured file, in bytes.
///
/// The live vault is a few kilobytes; this is four orders of magnitude above
/// it. It exists so a runaway or hostile write cannot turn "infinite history"
/// into an unbounded root-owned allocation, not because any real manifest
/// approaches it.
const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history denied: {0}")]
    Denied(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<AuditError> for HistoryError {
    fn from(error: AuditError) -> Self {
        match error {
            AuditError::Denied(message) => HistoryError::Denied(message),
            AuditError::Database(error) => HistoryError::Database(error),
            AuditError::Io(error) => HistoryError::Io(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A request to archive the pre-mutation state of the vault.
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    /// The transaction this state belongs to — the SAME uuid the audit ledger
    /// records for the operation, which is what joins the two stores.
    pub transaction: uuid::Uuid,
    /// The operation whose pre-state this is, e.g. `source-set`.
    pub operation: String,
    /// The manifest bytes as they were before the mutation.
    pub manifest: Vec<u8>,
    /// The dotenv bytes as they were before the mutation.
    pub dotenv: Vec<u8>,
    pub expected_uid: Option<u32>,
}

/// One secret's captured value within an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedValue {
    pub name: String,
    /// Digest of the captured value. Retained even after the value is
    /// destroyed, which is what makes a destroy provable rather than silent.
    pub value_sha256: String,
    /// The entry whose `destroy` removed this value, if any.
    pub destroyed_by: Option<i64>,
}

/// One archived pre-mutation state.
#[derive(Debug, Clone)]
pub struct Entry {
    pub sequence: i64,
    pub timestamp_ns: i64,
    pub transaction: uuid::Uuid,
    pub operation: String,
    pub manifest_sha256: String,
    /// Digest of the whole dotenv as captured, used to prove an exact
    /// reassembly from the per-name rows.
    pub env_sha256: String,
    /// Sorted by name; the order the chain hashes them in.
    pub values: Vec<CapturedValue>,
    pub previous_hash: String,
    pub entry_hash: String,
}

/// Result of a successful chain verification.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub entries: u64,
    /// How many captured values have been destroyed. Reported rather than
    /// hidden: a store with destroyed values is intact, not damaged, and the
    /// count is the difference an operator needs to see.
    pub destroyed: u64,
    pub hash: String,
}

// ---------------------------------------------------------------------------
// Dotenv grammar
// ---------------------------------------------------------------------------

/// Parse dotenv bytes into name/value pairs.
///
/// Uses the same `dotenv-ng` parser the engine's provider reads with, rather
/// than a hand-rolled `split('=')`. A second implementation of the grammar is a
/// second thing to keep in sync, and the one that drifts is the one that
/// silently mis-parses a quoted or multi-line value.
fn parse_dotenv(bytes: &[u8]) -> Result<BTreeMap<String, String>, HistoryError> {
    let map = dotenv::EnvLoader::with_reader(bytes)
        .sequence(dotenv::EnvSequence::InputOnly)
        .load()
        .map_err(|e| HistoryError::Denied(format!("dotenv is unparseable: {e}")))?;
    Ok(map
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// Render name/value pairs back into dotenv bytes.
///
/// Mirrors the engine's `serialize_dotenv`: sorted keys, `dotenv::render`, and
/// a trailing newline when non-empty. The duplication is three lines of
/// wrapper around the *same* renderer, and [`capture`]'s round-trip check is
/// what proves the wrapper still agrees with the engine's — a divergence fails
/// a capture instead of corrupting a restore.
pub fn render_dotenv(pairs: &BTreeMap<String, String>) -> Result<Vec<u8>, HistoryError> {
    let mut out = dotenv::render(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_err(|e| HistoryError::Denied(format!("dotenv cannot be rendered: {e}")))?;
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out.into_bytes())
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

fn ensure_schema(conn: &Connection) -> Result<(), HistoryError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (\
           sequence        INTEGER PRIMARY KEY AUTOINCREMENT,\
           timestamp_ns    INTEGER NOT NULL,\
           transaction_id  TEXT    NOT NULL,\
           operation       TEXT    NOT NULL,\
           manifest_blob   BLOB    NOT NULL,\
           manifest_sha256 TEXT    NOT NULL,\
           env_sha256      TEXT    NOT NULL,\
           previous_hash   TEXT    NOT NULL,\
           entry_hash      TEXT    NOT NULL UNIQUE\
         ) STRICT;\
         CREATE TABLE IF NOT EXISTS captured_values (\
           sequence     INTEGER NOT NULL REFERENCES entries(sequence),\
           name         TEXT    NOT NULL,\
           value_blob   BLOB,\
           value_sha256 TEXT    NOT NULL,\
           destroyed_by INTEGER REFERENCES entries(sequence),\
           PRIMARY KEY (sequence, name),\
           CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))\
         ) STRICT;\
         CREATE TABLE IF NOT EXISTS head (\
           singleton  INTEGER PRIMARY KEY CHECK (singleton = 1),\
           sequence   INTEGER NOT NULL,\
           entry_hash TEXT    NOT NULL\
         ) STRICT;",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Canonical serialisation
// ---------------------------------------------------------------------------

/// Canonical JSON for an entry: everything that identifies it except
/// `entry_hash` itself, and except `destroyed_by`.
///
/// `destroyed_by` is excluded deliberately. Destroying a value must not
/// invalidate the chain — the whole point of retaining `value_sha256` is that
/// the entry still verifies afterwards and still proves what was destroyed.
/// The schema's `CHECK` is what keeps `destroyed_by` honest instead: a row
/// holds its blob if and only if it is not destroyed, so a forged
/// `destroyed_by` on a row that kept its bytes fails verification.
fn canonical_entry_json(entry: &Entry) -> String {
    let mut map = BTreeMap::new();
    map.insert("sequence", serde_json::Value::Number(entry.sequence.into()));
    map.insert(
        "timestamp_ns",
        serde_json::Value::Number(entry.timestamp_ns.into()),
    );
    map.insert(
        "transaction",
        serde_json::Value::String(entry.transaction.to_string()),
    );
    map.insert(
        "operation",
        serde_json::Value::String(entry.operation.clone()),
    );
    map.insert(
        "manifest_sha256",
        serde_json::Value::String(entry.manifest_sha256.clone()),
    );
    map.insert(
        "env_sha256",
        serde_json::Value::String(entry.env_sha256.clone()),
    );
    map.insert("values", {
        // Name and digest only. Binding the digests is what stops a stored
        // value being swapped for a different one without detection.
        let values: Vec<serde_json::Value> = entry
            .values
            .iter()
            .map(|v| {
                let mut pair = serde_json::Map::new();
                pair.insert("name".into(), serde_json::Value::String(v.name.clone()));
                pair.insert(
                    "value_sha256".into(),
                    serde_json::Value::String(v.value_sha256.clone()),
                );
                serde_json::Value::Object(pair)
            })
            .collect();
        serde_json::Value::Array(values)
    });
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

// ---------------------------------------------------------------------------
// Chain verification (in-transaction)
// ---------------------------------------------------------------------------

/// Verify the chain inside an active read transaction.
///
/// Returns `(tip hash, entry count, destroyed value count)`.
fn verify_rows(conn: &Connection) -> Result<(String, i64, i64), HistoryError> {
    let mut previous = ZERO_HASH.to_string();
    let mut count: i64 = 0;
    let mut destroyed: i64 = 0;

    let mut stmt = conn.prepare(
        "SELECT sequence, timestamp_ns, transaction_id, operation,\
                manifest_blob, manifest_sha256, env_sha256, previous_hash, entry_hash \
         FROM entries ORDER BY sequence",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;

    let mut values_stmt = conn.prepare(
        "SELECT name, value_blob, value_sha256, destroyed_by \
         FROM captured_values WHERE sequence = ?1 ORDER BY name",
    )?;

    for row in rows {
        let (
            sequence,
            timestamp_ns,
            transaction_id,
            operation,
            manifest_blob,
            manifest_sha256,
            env_sha256,
            stored_previous,
            entry_hash,
        ) = row?;

        // The stored manifest must still be the bytes its digest claims. The
        // chain binds the digest; this is what binds the digest to the blob.
        if sha256_hex(&manifest_blob) != manifest_sha256 {
            return Err(HistoryError::Denied(
                "history manifest does not match its digest".into(),
            ));
        }

        let transaction = uuid::Uuid::parse_str(&transaction_id).map_err(|e| {
            HistoryError::Denied(format!("invalid transaction UUID in history: {e}"))
        })?;

        let mut values = Vec::new();
        let value_rows = values_stmt.query_map([sequence], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for value_row in value_rows {
            let (name, value_blob, value_sha256, destroyed_by) = value_row?;
            match (&value_blob, destroyed_by) {
                // A live value must be the bytes its digest claims.
                (Some(blob), None) => {
                    if sha256_hex(blob) != value_sha256 {
                        return Err(HistoryError::Denied(format!(
                            "history value for {name} does not match its digest"
                        )));
                    }
                }
                // A destroyed value keeps only its digest, by design.
                (None, Some(_)) => destroyed += 1,
                // The schema's CHECK forbids both remaining combinations, so
                // reaching one means the file was edited outside this module.
                _ => {
                    return Err(HistoryError::Denied(format!(
                        "history value for {name} is neither live nor destroyed"
                    )));
                }
            }
            values.push(CapturedValue {
                name,
                value_sha256,
                destroyed_by,
            });
        }

        let entry = Entry {
            sequence,
            timestamp_ns,
            transaction,
            operation,
            manifest_sha256,
            env_sha256,
            values,
            previous_hash: stored_previous.clone(),
            entry_hash: entry_hash.clone(),
        };

        if stored_previous != previous || entry_hash != compute_entry_hash(&previous, &entry) {
            return Err(HistoryError::Denied("history chain is invalid".into()));
        }

        previous = entry_hash;
        count = sequence;
    }

    // A value row pointing at no entry would be invisible to the loop above,
    // so it is checked separately rather than assumed impossible.
    let orphans: i64 = conn.query_row(
        "SELECT COUNT(*) FROM captured_values \
         WHERE sequence NOT IN (SELECT sequence FROM entries)",
        [],
        |row| row.get(0),
    )?;
    if orphans != 0 {
        return Err(HistoryError::Denied(
            "history holds values belonging to no entry".into(),
        ));
    }

    let head: Option<(i64, String)> = conn
        .query_row(
            "SELECT sequence, entry_hash FROM head WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let expected_head = if count == 0 {
        None
    } else {
        Some((count, previous.clone()))
    };
    if head != expected_head {
        return Err(HistoryError::Denied("history head is invalid".into()));
    }

    Ok((previous, count, destroyed))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Verify the full history chain, returning the tip hash and counts.
///
/// A store that does not exist yet verifies as empty, so a boundary installed
/// before this module existed is not reported as damaged.
pub fn verify(directory: &Path, expected_uid: Option<u32>) -> Result<VerifyResult, HistoryError> {
    verify_with(directory, expected_uid, VerifyMode::ReadWrite)
}

/// Verify the history without touching it.
///
/// For callers that promise not to mutate state — `drift`, and so `doctor`.
/// Same reasoning as [`crate::audit::verify_read_only`].
pub fn verify_read_only(
    directory: &Path,
    expected_uid: Option<u32>,
) -> Result<VerifyResult, HistoryError> {
    verify_with(directory, expected_uid, VerifyMode::ReadOnly)
}

fn verify_with(
    directory: &Path,
    expected_uid: Option<u32>,
    mode: VerifyMode,
) -> Result<VerifyResult, HistoryError> {
    if !directory.join(DB_NAME).exists() {
        audit::require_protected_dir(directory, expected_uid)?;
        return Ok(VerifyResult {
            entries: 0,
            destroyed: 0,
            hash: ZERO_HASH.into(),
        });
    }

    let conn = audit::open_protected_db(directory, DB_NAME, expected_uid, mode)?;
    let result = (|| -> Result<VerifyResult, HistoryError> {
        if mode == VerifyMode::ReadWrite {
            ensure_schema(&conn)?;
        }
        conn.execute(
            match mode {
                VerifyMode::ReadWrite => "BEGIN IMMEDIATE",
                VerifyMode::ReadOnly => "BEGIN",
            },
            [],
        )?;
        let (hash, count, destroyed) = verify_rows(&conn)?;

        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(HistoryError::Denied(
                "history integrity check failed".into(),
            ));
        }

        conn.execute("COMMIT", [])?;
        Ok(VerifyResult {
            entries: count as u64,
            destroyed: destroyed as u64,
            hash,
        })
    })();

    match result {
        Ok(r) => Ok(r),
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

/// Archive one pre-mutation state.
///
/// Validates everything it can before opening the database, and refuses a
/// dotenv it cannot reproduce byte for byte. Storing a snapshot that cannot be
/// restored is worse than storing none: it is a promise of recoverability that
/// only fails when it is finally needed.
pub fn capture(directory: &Path, request: CaptureRequest) -> Result<Entry, HistoryError> {
    if request.manifest.len() > MAX_CAPTURE_BYTES || request.dotenv.len() > MAX_CAPTURE_BYTES {
        return Err(HistoryError::Denied(
            "captured file exceeds the history size limit".into(),
        ));
    }
    if request.operation.is_empty()
        || !request
            .operation
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(HistoryError::Denied("invalid operation".into()));
    }

    let pairs = parse_dotenv(&request.dotenv)?;

    // The round-trip guard. Rendering what was just parsed must reproduce the
    // original bytes, or the per-name rows are not a faithful decomposition of
    // this file and a whole-vault restore from them would silently differ.
    let rendered = render_dotenv(&pairs)?;
    if rendered != request.dotenv {
        return Err(HistoryError::Denied(
            "dotenv does not round-trip through the renderer; refusing to capture a \
             snapshot that could not be restored exactly"
                .into(),
        ));
    }

    let manifest_sha256 = sha256_hex(&request.manifest);
    let env_sha256 = sha256_hex(&request.dotenv);

    let conn = audit::open_protected_db(
        directory,
        DB_NAME,
        request.expected_uid,
        VerifyMode::ReadWrite,
    )?;

    let result = (|| -> Result<Entry, HistoryError> {
        ensure_schema(&conn)?;
        conn.execute("BEGIN IMMEDIATE", [])?;

        let (previous_hash, count, _) = verify_rows(&conn)?;
        let sequence = count + 1;

        let timestamp_ns = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| HistoryError::Denied(format!("clock error: {e}")))?;
            now.as_nanos() as i64
        };

        let values: Vec<CapturedValue> = pairs
            .iter()
            .map(|(name, value)| CapturedValue {
                name: name.clone(),
                value_sha256: sha256_hex(value.as_bytes()),
                destroyed_by: None,
            })
            .collect();

        let mut entry = Entry {
            sequence,
            timestamp_ns,
            transaction: request.transaction,
            operation: request.operation.clone(),
            manifest_sha256,
            env_sha256,
            values,
            previous_hash: previous_hash.clone(),
            entry_hash: String::new(),
        };
        entry.entry_hash = compute_entry_hash(&previous_hash, &entry);

        conn.execute(
            "INSERT INTO entries (\
                sequence, timestamp_ns, transaction_id, operation,\
                manifest_blob, manifest_sha256, env_sha256, previous_hash, entry_hash\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                entry.sequence,
                entry.timestamp_ns,
                entry.transaction.to_string(),
                entry.operation,
                request.manifest,
                entry.manifest_sha256,
                entry.env_sha256,
                entry.previous_hash,
                entry.entry_hash,
            ],
        )?;

        for value in &entry.values {
            let plain = pairs
                .get(&value.name)
                .expect("every captured value came from these pairs");
            conn.execute(
                "INSERT INTO captured_values (sequence, name, value_blob, value_sha256, destroyed_by) \
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                rusqlite::params![
                    entry.sequence,
                    value.name,
                    plain.as_bytes(),
                    value.value_sha256,
                ],
            )?;
        }

        conn.execute(
            "INSERT INTO head (singleton, sequence, entry_hash) VALUES (1, ?1, ?2) \
             ON CONFLICT(singleton) DO UPDATE SET \
             sequence = excluded.sequence, entry_hash = excluded.entry_hash",
            rusqlite::params![entry.sequence, entry.entry_hash],
        )?;

        conn.execute("COMMIT", [])?;
        Ok(entry)
    })();

    match result {
        Ok(entry) => Ok(entry),
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// A vault directory the protected-metadata checks accept.
    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    const MANIFEST: &[u8] = b"[project]\nname = \"fixture\"\nrevision = \"1.0\"\n";

    fn request(transaction: uuid::Uuid, dotenv: &[u8]) -> CaptureRequest {
        CaptureRequest {
            transaction,
            operation: "source-set".into(),
            manifest: MANIFEST.to_vec(),
            dotenv: dotenv.to_vec(),
            expected_uid: None,
        }
    }

    #[test]
    fn an_absent_store_verifies_as_empty_rather_than_damaged() {
        // A boundary installed before this module existed must not report its
        // missing history as corruption.
        let dir = vault();
        let result = verify(dir.path(), None).unwrap();
        assert_eq!(result.entries, 0);
        assert_eq!(result.hash, ZERO_HASH);
    }

    #[test]
    fn a_capture_round_trips_to_the_exact_bytes_it_was_given() {
        let dir = vault();
        let env = b"ALPHA=one\nBETA=two\n";
        let entry = capture(dir.path(), request(uuid::Uuid::new_v4(), env)).unwrap();

        assert_eq!(entry.sequence, 1);
        assert_eq!(entry.env_sha256, sha256_hex(env));
        let names: Vec<&str> = entry.values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["ALPHA", "BETA"]);

        let pairs = parse_dotenv(env).unwrap();
        assert_eq!(render_dotenv(&pairs).unwrap(), env);
    }

    #[test]
    fn a_dotenv_that_does_not_round_trip_is_refused_at_capture() {
        // The guarantee this module sells is that a stored snapshot restores
        // exactly. A file the renderer normalises differently cannot honour
        // that, so it must fail here rather than at the restore that needs it.
        let dir = vault();
        // Extra blank lines and a comment: parseable, but not what the
        // renderer emits, so the reassembly would not be byte-identical.
        let env = b"# a comment the renderer does not keep\nALPHA=one\n\n\nBETA=two\n";
        let err = capture(dir.path(), request(uuid::Uuid::new_v4(), env)).unwrap_err();
        assert!(
            err.to_string().contains("round-trip"),
            "unexpected error: {err}"
        );
        // And nothing was stored.
        assert_eq!(verify(dir.path(), None).unwrap().entries, 0);
    }

    #[test]
    fn successive_captures_chain_and_verify() {
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\nB=2\n")).unwrap();
        let third = capture(dir.path(), request(uuid::Uuid::new_v4(), b"B=2\n")).unwrap();

        let result = verify(dir.path(), None).unwrap();
        assert_eq!(result.entries, 3);
        assert_eq!(result.destroyed, 0);
        assert_eq!(result.hash, third.entry_hash);
        assert_ne!(result.hash, ZERO_HASH);
    }

    #[test]
    fn the_first_entry_links_to_the_genesis_hash() {
        let dir = vault();
        let entry = capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();
        assert_eq!(entry.previous_hash, ZERO_HASH);
    }

    #[test]
    fn editing_a_stored_value_is_detected() {
        // The reason the chain binds each value's digest rather than only the
        // whole-file digest: a swapped value would otherwise verify.
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute(
            "UPDATE captured_values SET value_blob = ?1 WHERE name = 'A'",
            rusqlite::params![b"tampered".as_slice()],
        )
        .unwrap();
        drop(conn);

        let err = verify(dir.path(), None).unwrap_err();
        assert!(
            err.to_string().contains("does not match its digest"),
            "{err}"
        );
    }

    #[test]
    fn editing_a_stored_manifest_is_detected() {
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute(
            "UPDATE entries SET manifest_blob = ?1 WHERE sequence = 1",
            rusqlite::params![b"[project]\nname = \"other\"\n".as_slice()],
        )
        .unwrap();
        drop(conn);

        let err = verify(dir.path(), None).unwrap_err();
        assert!(
            err.to_string()
                .contains("manifest does not match its digest"),
            "{err}"
        );
    }

    #[test]
    fn removing_an_entry_from_the_middle_breaks_the_chain() {
        let dir = vault();
        for env in [
            b"A=1\n".as_slice(),
            b"A=2\n".as_slice(),
            b"A=3\n".as_slice(),
        ] {
            capture(dir.path(), request(uuid::Uuid::new_v4(), env)).unwrap();
        }

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute("DELETE FROM captured_values WHERE sequence = 2", [])
            .unwrap();
        conn.execute("DELETE FROM entries WHERE sequence = 2", [])
            .unwrap();
        drop(conn);

        let err = verify(dir.path(), None).unwrap_err();
        assert!(err.to_string().contains("chain is invalid"), "{err}");
    }

    #[test]
    fn back_dating_an_entry_breaks_the_chain() {
        // The timestamp is inside the hash, so history cannot be re-dated in
        // place to make a restore look older or newer than it was.
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute("UPDATE entries SET timestamp_ns = 1 WHERE sequence = 1", [])
            .unwrap();
        drop(conn);

        let err = verify(dir.path(), None).unwrap_err();
        assert!(err.to_string().contains("chain is invalid"), "{err}");
    }

    #[test]
    fn a_value_belonging_to_no_entry_is_detected() {
        // Such a row is invisible to the per-entry loop, so it needs its own
        // check rather than being assumed impossible.
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute(
            "INSERT INTO captured_values (sequence, name, value_blob, value_sha256, destroyed_by) \
             VALUES (99, 'GHOST', ?1, ?2, NULL)",
            rusqlite::params![b"x".as_slice(), sha256_hex(b"x")],
        )
        .unwrap();
        drop(conn);

        let err = verify(dir.path(), None).unwrap_err();
        assert!(err.to_string().contains("belonging to no entry"), "{err}");
    }

    #[test]
    fn a_row_that_is_neither_live_nor_destroyed_is_refused_by_the_schema() {
        // The CHECK is what keeps `destroyed_by` honest without putting it in
        // the hash: bytes present and a destroy stamp cannot coexist.
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();

        let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
        let err = conn
            .execute(
                "UPDATE captured_values SET destroyed_by = 1 WHERE name = 'A'",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("CHECK"), "{err}");
    }

    #[test]
    fn an_empty_dotenv_is_capturable() {
        // A fresh install's vault has one, and it is exactly the state the
        // truncation incident produced — the one most worth being able to
        // recognise in history.
        let dir = vault();
        let entry = capture(dir.path(), request(uuid::Uuid::new_v4(), b"")).unwrap();
        assert!(entry.values.is_empty());
        assert_eq!(verify(dir.path(), None).unwrap().entries, 1);
    }

    #[test]
    fn an_oversized_capture_is_refused_before_the_database_is_opened() {
        let dir = vault();
        let mut request = request(uuid::Uuid::new_v4(), b"A=1\n");
        request.manifest = vec![b'x'; MAX_CAPTURE_BYTES + 1];
        let err = capture(dir.path(), request).unwrap_err();
        assert!(err.to_string().contains("size limit"), "{err}");
        assert!(!dir.path().join(DB_NAME).exists());
    }

    #[test]
    fn the_store_is_created_private_to_the_vault_owner() {
        let dir = vault();
        capture(dir.path(), request(uuid::Uuid::new_v4(), b"A=1\n")).unwrap();
        let mode = fs::metadata(dir.path().join(DB_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "history must not be readable beyond its owner");
    }

    #[test]
    fn a_quoted_value_survives_capture_and_reassembly() {
        // Values that need quoting are exactly where a hand-rolled parser
        // would differ from the engine's, so the grammar is shared instead.
        let dir = vault();
        let pairs: BTreeMap<String, String> = [
            ("PLAIN".to_string(), "simple".to_string()),
            ("SPACED".to_string(), "two words".to_string()),
            ("QUOTED".to_string(), "{\"json\":\"yes\"}".to_string()),
        ]
        .into_iter()
        .collect();
        let env = render_dotenv(&pairs).unwrap();

        let entry = capture(dir.path(), request(uuid::Uuid::new_v4(), &env)).unwrap();
        assert_eq!(entry.values.len(), 3);
        assert_eq!(parse_dotenv(&env).unwrap(), pairs);
    }
}
