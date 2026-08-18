import re
import sys

with open("secretspec/src/provider/sqlite.rs", "r") as f:
    content = f.read()

# 1. Add `history` to `SqliteConfig`
content = content.replace("""pub struct SqliteConfig {
    /// Path to the SQLite database file.
    pub path: PathBuf,
}""", """pub struct SqliteConfig {
    /// Path to the SQLite database file.
    pub path: PathBuf,
    /// Whether history tracking is enabled.
    #[serde(default)]
    pub history: bool,
}""")

# 2. Modify `try_from`
old_try_from = """        if !url.username().is_empty() || url.password().is_some() || url.has_query() {
            return Err(operation_error(
                "sqlite provider URIs take only a database path; user information and query \\
                 options are not supported",
            ));
        }"""
new_try_from = """        if !url.username().is_empty() || url.password().is_some() {
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
        }"""
content = content.replace(old_try_from, new_try_from)

content = content.replace("""        Ok(Self {
            path: PathBuf::from(path),
        })""", """        Ok(Self {
            path: PathBuf::from(path),
            history,
        })""")

# 3. Add `sha2` and other imports
content = content.replace("use std::time::Duration;", "use std::time::Duration;\nuse sha2::{Sha256, Digest};\nuse std::collections::BTreeMap;")

# 4. Modify `connection`
old_connection = """        conn.execute_batch(
            "PRAGMA journal_mode=DELETE;\\
             PRAGMA synchronous=FULL;\\
             PRAGMA trusted_schema=OFF;\\
             CREATE TABLE IF NOT EXISTS secrets (\\
                 item TEXT PRIMARY KEY,\\
                 value TEXT NOT NULL\\
             ) STRICT;",
        )
        .map_err(|error| {"""
new_connection = """        let mut init_sql = String::from(
            "PRAGMA journal_mode=DELETE;\\
             PRAGMA synchronous=FULL;\\
             PRAGMA trusted_schema=OFF;\\
             CREATE TABLE IF NOT EXISTS secrets (\\
                 item TEXT PRIMARY KEY,\\
                 value TEXT NOT NULL\\
             ) STRICT;"
        );
        if self.config.history {
            init_sql.push_str(
                "CREATE TABLE IF NOT EXISTS entries (\\
                     sequence INTEGER PRIMARY KEY AUTOINCREMENT,\\
                     timestamp_ns INTEGER NOT NULL,\\
                     operation TEXT NOT NULL,\\
                     previous_hash TEXT NOT NULL,\\
                     entry_hash TEXT NOT NULL UNIQUE\\
                 ) STRICT;\\
                 CREATE TABLE IF NOT EXISTS captured_values (\\
                     sequence INTEGER NOT NULL REFERENCES entries(sequence),\\
                     item TEXT NOT NULL,\\
                     value_blob BLOB,\\
                     value_sha256 TEXT NOT NULL,\\
                     destroyed_by INTEGER REFERENCES entries(sequence),\\
                     PRIMARY KEY (sequence, item),\\
                     CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))\\
                 ) STRICT;\\
                 CREATE TABLE IF NOT EXISTS head (\\
                     singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\\
                     sequence INTEGER NOT NULL,\\
                     entry_hash TEXT NOT NULL\\
                 ) STRICT;"
            );
        }
        conn.execute_batch(&init_sql)
        .map_err(|error| {"""
content = content.replace(old_connection, new_connection)

# 5. Modify `set` and `delete`
old_set = """    fn set(&self, addr: Address<'_>, value: &SecretString) -> Result<()> {
        self.check_writable(addr)?;
        let item = super::flat_item(self, addr)?;
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO secrets (item, value) VALUES (?1, ?2) \\
             ON CONFLICT(item) DO UPDATE SET value = excluded.value",
            params![item.as_ref(), value.expose_secret()],
        )
        .map_err(|error| {
            operation_error(format!(
                "failed to write sqlite provider entry '{item}': {error}"
            ))
        })?;
        Ok(())
    }"""
new_set = """    fn set(&self, addr: Address<'_>, value: &SecretString) -> Result<()> {
        self.check_writable(addr)?;
        let item = super::flat_item(self, addr)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| {
            operation_error(format!("failed to start sqlite transaction: {error}"))
        })?;
        tx.execute(
            "INSERT INTO secrets (item, value) VALUES (?1, ?2) \\
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
    }"""
content = content.replace(old_set, new_set)

old_delete = """    fn delete(&self, addr: Address<'_>) -> Result<bool> {
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
    }"""
new_delete = """    fn delete(&self, addr: Address<'_>) -> Result<bool> {
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
    }"""
content = content.replace(old_delete, new_delete)

# 6. Add history helper functions at the end before tests
helpers = """
// ---------------------------------------------------------------------------
// History Capture
// ---------------------------------------------------------------------------

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
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
    sha256_hex(format!("{previous_hash}\\n{canonical}").as_bytes())
}

fn capture_history(conn: &Connection, operation: &str) -> Result<()> {
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
        "INSERT INTO entries (sequence, timestamp_ns, operation, previous_hash, entry_hash) \\
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
        conn.execute(
            "INSERT INTO captured_values (sequence, item, value_blob, value_sha256, destroyed_by) \\
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![
                entry.sequence,
                val.item,
                plain.as_bytes(),
                val.value_sha256,
            ],
        )
        .map_err(|e| operation_error(e.to_string()))?;
    }

    conn.execute(
        "INSERT INTO head (singleton, sequence, entry_hash) VALUES (1, ?1, ?2) \\
         ON CONFLICT(singleton) DO UPDATE SET \\
         sequence = excluded.sequence, entry_hash = excluded.entry_hash",
        params![entry.sequence, entry.entry_hash],
    )
    .map_err(|e| operation_error(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
"""
content = content.replace("#[cfg(test)]", helpers)

# 7. Update tests to check missing database
old_test_missing_path = """        for uri in [
            "sqlite://",
            "sqlite://./secrets.db?history=on",
            "sqlite://user:pass@./secrets.db",
        ] {"""
new_test_missing_path = """        for uri in [
            "sqlite://",
            "sqlite://./secrets.db?invalid=on",
            "sqlite://user:pass@./secrets.db",
        ] {"""
content = content.replace(old_test_missing_path, new_test_missing_path)

with open("secretspec/src/provider/sqlite.rs", "w") as f:
    f.write(content)
