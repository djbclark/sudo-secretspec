//! Infinite secret history for the sudo-secretspec privilege boundary.
//!
//! The boundary snapshots the manifest before every mutating operation.
//! (Dotenv and values are now managed by the provider, not the boundary).
//!
//! ## Chain
//!
//! Entries are hash-chained exactly as [`crate::audit`] chains events.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::audit::{self, AuditError, VerifyMode, ZERO_HASH, sha256_hex};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DB_NAME: &str = "broker-history.sqlite3";

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

#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub transaction: uuid::Uuid,
    pub operation: String,
    pub manifest: Vec<u8>,
    pub expected_uid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub sequence: i64,
    pub timestamp_ns: i64,
    pub transaction: uuid::Uuid,
    pub operation: String,
    pub manifest_sha256: String,
    pub previous_hash: String,
    pub entry_hash: String,
}

#[derive(Debug, Clone)]
pub struct EntrySummary {
    pub sequence: i64,
    pub timestamp_ns: i64,
    pub transaction: uuid::Uuid,
    pub operation: String,
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub entries: u64,
    pub hash: String,
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
           previous_hash   TEXT    NOT NULL,\
           entry_hash      TEXT    NOT NULL UNIQUE\
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

fn verify_rows(conn: &Connection) -> Result<(String, i64), HistoryError> {
    let mut previous = ZERO_HASH.to_string();
    let mut count: i64 = 0;

    let mut stmt = conn.prepare(
        "SELECT sequence, timestamp_ns, transaction_id, operation,\
                manifest_blob, manifest_sha256, previous_hash, entry_hash \
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
        ))
    })?;

    for row in rows {
        let (
            sequence,
            timestamp_ns,
            transaction_id,
            operation,
            manifest_blob,
            manifest_sha256,
            stored_previous,
            entry_hash,
        ) = row?;

        if sha256_hex(&manifest_blob) != manifest_sha256 {
            return Err(HistoryError::Denied(
                "history manifest does not match its digest".into(),
            ));
        }

        let transaction = uuid::Uuid::parse_str(&transaction_id).map_err(|e| {
            HistoryError::Denied(format!("invalid transaction UUID in history: {e}"))
        })?;

        let entry = Entry {
            sequence,
            timestamp_ns,
            transaction,
            operation,
            manifest_sha256,
            previous_hash: stored_previous.clone(),
            entry_hash: entry_hash.clone(),
        };

        if stored_previous != previous || entry_hash != compute_entry_hash(&previous, &entry) {
            return Err(HistoryError::Denied("history chain is invalid".into()));
        }

        previous = entry_hash;
        count = sequence;
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

    Ok((previous, count))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn verify(directory: &Path, expected_uid: Option<u32>) -> Result<VerifyResult, HistoryError> {
    verify_with(directory, expected_uid, VerifyMode::ReadWrite)
}

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
        let (hash, count) = verify_rows(&conn)?;

        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(HistoryError::Denied(
                "history integrity check failed".into(),
            ));
        }

        conn.execute("COMMIT", [])?;
        Ok(VerifyResult {
            entries: count as u64,
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

pub fn list(
    directory: &Path,
    name: Option<&str>,
    expected_uid: Option<u32>,
) -> Result<Vec<EntrySummary>, HistoryError> {
    // Note: The `name` parameter is preserved for API compatibility,
    // but the backend no longer tracks value names.
    let _ = name;

    if !directory.join(DB_NAME).exists() {
        audit::require_protected_dir(directory, expected_uid)?;
        return Ok(Vec::new());
    }

    let conn = audit::open_protected_db(directory, DB_NAME, expected_uid, VerifyMode::ReadOnly)?;
    let result = (|| -> Result<Vec<EntrySummary>, HistoryError> {
        conn.execute("BEGIN", [])?;
        verify_rows(&conn)?;

        let mut stmt = conn.prepare(
            "SELECT sequence, timestamp_ns, transaction_id, operation \
             FROM entries ORDER BY sequence",
        )?;
        let rows: Vec<(i64, i64, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?;

        let mut out = Vec::new();
        for (sequence, timestamp_ns, transaction_id, operation) in rows {
            let transaction = uuid::Uuid::parse_str(&transaction_id).map_err(|e| {
                HistoryError::Denied(format!("invalid transaction UUID in history: {e}"))
            })?;
            out.push(EntrySummary {
                sequence,
                timestamp_ns,
                transaction,
                operation,
            });
        }

        conn.execute("COMMIT", [])?;
        Ok(out)
    })();

    match result {
        Ok(entries) => Ok(entries),
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

pub fn capture(directory: &Path, request: CaptureRequest) -> Result<Entry, HistoryError> {
    if request.manifest.len() > MAX_CAPTURE_BYTES {
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

    let manifest_sha256 = sha256_hex(&request.manifest);

    let conn = audit::open_protected_db(
        directory,
        DB_NAME,
        request.expected_uid,
        VerifyMode::ReadWrite,
    )?;

    let result = (|| -> Result<Entry, HistoryError> {
        ensure_schema(&conn)?;
        conn.execute("BEGIN IMMEDIATE", [])?;

        let (previous_hash, count) = verify_rows(&conn)?;
        let sequence = count + 1;

        let timestamp_ns = {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| HistoryError::Denied(format!("clock error: {e}")))?;
            now.as_nanos() as i64
        };

        let mut entry = Entry {
            sequence,
            timestamp_ns,
            transaction: request.transaction,
            operation: request.operation.clone(),
            manifest_sha256,
            previous_hash: previous_hash.clone(),
            entry_hash: String::new(),
        };
        entry.entry_hash = compute_entry_hash(&previous_hash, &entry);

        conn.execute(
            "INSERT INTO entries (\
                sequence, timestamp_ns, transaction_id, operation,\
                manifest_blob, manifest_sha256, previous_hash, entry_hash\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                entry.sequence,
                entry.timestamp_ns,
                entry.transaction.to_string(),
                entry.operation,
                request.manifest,
                entry.manifest_sha256,
                entry.previous_hash,
                entry.entry_hash,
            ],
        )?;

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

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    const MANIFEST: &[u8] = b"[project]\nname = \"fixture\"\nrevision = \"1.0\"\n";

    fn request(transaction: uuid::Uuid) -> CaptureRequest {
        CaptureRequest {
            transaction,
            operation: "source-set".into(),
            manifest: MANIFEST.to_vec(),
            expected_uid: None,
        }
    }

    #[test]
    fn an_absent_store_verifies_as_empty_rather_than_damaged() {
        let dir = vault();
        let result = verify(dir.path(), None).unwrap();
        assert_eq!(result.entries, 0);
        assert_eq!(result.hash, ZERO_HASH);
    }

    #[test]
    fn a_capture_stores_the_manifest_and_advances_the_chain() {
        let dir = vault();
        let entry = capture(dir.path(), request(uuid::Uuid::new_v4())).unwrap();

        assert_eq!(entry.sequence, 1);
        assert_eq!(entry.manifest_sha256, sha256_hex(MANIFEST));

        let result = verify(dir.path(), None).unwrap();
        assert_eq!(result.entries, 1);
        assert_eq!(result.hash, entry.entry_hash);
    }
}
