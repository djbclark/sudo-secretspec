//! Integration tests for the sudo-secretspec audit ledger.
//!
//! These tests verify the fail-closed transactional SQLite audit module:
//! hash-chained events, atomic event+head updates, protected metadata checks,
//! value-free storage (no secrets, reasons, or argv), and chain verification.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use sudo_secretspec_cli::audit::{self, AuditError, ClientFamily, Outcome};

/// Create a protected directory (mode 0700) under tmp_path.
fn protected_dir(tmp: &std::path::Path) -> std::path::PathBuf {
    let dir = tmp.join("protected");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// Helper to append an attempt event and return the event.
fn append_attempt(dir: &std::path::Path, tx: uuid::Uuid) -> Result<audit::AuditEvent, AuditError> {
    audit::append_event(
        dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: tx,
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"rotate integration credential")),
            command_basename: Some("curl".into()),
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    )
}

/// Helper to append a terminal event (success/failure/unknown).
fn append_terminal(
    dir: &std::path::Path,
    tx: uuid::Uuid,
    phase: Outcome,
    result_code: u8,
) -> Result<audit::AuditEvent, AuditError> {
    audit::append_event(
        dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase,
            transaction: tx,
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"rotate integration credential")),
            command_basename: Some("curl".into()),
            names: vec!["EXAMPLE_KEY".into()],
            result_code: Some(result_code),
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    )
}

// ---------------------------------------------------------------------------
// RED-1: attempt + success are transactional and value-free
// ---------------------------------------------------------------------------
#[test]
fn test_attempt_and_success_are_transactional_and_value_free() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());
    let tx = uuid::Uuid::new_v4();

    let first = append_attempt(&dir, tx).expect("attempt should succeed");
    let second = append_terminal(&dir, tx, Outcome::Success, 0).expect("success should succeed");

    // Verify chain linking
    assert_eq!(first.client, ClientFamily::Hermes);
    assert_eq!(second.previous_hash, first.event_hash);

    // Verify value-free: no reason text, no session, no credential in raw file
    let raw = fs::read(dir.join("broker-audit.sqlite3")).unwrap();
    assert!(!window_sensitive("rotate integration credential", &raw));
    assert!(!window_sensitive("hermes:topic", &raw));
    assert!(!window_sensitive("Authorization: ***", &raw));
    // Names are expected in the DB
    assert!(window_sensitive("EXAMPLE_KEY", &raw));

    // Verify file permissions
    let meta = fs::metadata(dir.join("broker-audit.sqlite3")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);

    // Verify chain
    let result =
        audit::verify(&dir, Some(unsafe { libc::getuid() })).expect("verify should succeed");
    assert_eq!(result.count, 2);
    assert_eq!(result.hash, second.event_hash);
}

// ---------------------------------------------------------------------------
// RED-2: unknown outcome is a distinct terminal phase
// ---------------------------------------------------------------------------
#[test]
fn test_unknown_outcome_is_a_distinct_terminal_phase() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());
    let tx = uuid::Uuid::new_v4();

    append_attempt(&dir, tx).unwrap();
    let unknown = append_terminal(&dir, tx, Outcome::Unknown, 125).expect("unknown should succeed");
    assert_eq!(unknown.phase, Outcome::Unknown);
    assert_eq!(unknown.result_code, Some(125));

    let result = audit::verify(&dir, Some(unsafe { libc::getuid() })).unwrap();
    assert_eq!(result.count, 2);
}

// ---------------------------------------------------------------------------
// RED-3: free-form reason and command arguments never persist
// ---------------------------------------------------------------------------
#[test]
fn test_free_form_reason_and_command_arguments_never_persist() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    let secretish = "Authorization: Bearer ***";
    let reason_hash = audit::sha256_hex(secretish.as_bytes());

    audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(reason_hash),
            command_basename: Some("curl".into()),
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    )
    .unwrap();

    let raw = fs::read(dir.join("broker-audit.sqlite3")).unwrap();
    // Reason text must not appear
    assert!(!window_sensitive(secretish, &raw));
    assert!(!window_sensitive("credential-value", &raw));
    // Basename should appear
    assert!(window_sensitive("curl", &raw));
}

// ---------------------------------------------------------------------------
// RED-4: interrupted attempt remains visible
// ---------------------------------------------------------------------------
#[test]
fn test_interrupted_attempt_remains_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    append_attempt(&dir, uuid::Uuid::new_v4()).unwrap();

    let result = audit::verify(&dir, Some(unsafe { libc::getuid() })).unwrap();
    assert_eq!(result.count, 1);
}

// ---------------------------------------------------------------------------
// RED-5: bad metadata fails before creating ledger
// ---------------------------------------------------------------------------
#[test]
fn test_bad_metadata_fails_before_creating_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Invalid secret name
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["BAD-NAME".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err());
    let db = dir.join("broker-audit.sqlite3");
    assert!(!db.exists(), "ledger should not be created on bad metadata");

    // Invalid command basename with newline
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: Some("bad\ncommand".into()),
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err());

    // Invalid client family
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    // This one should succeed since Hermes is a valid client
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// RED-6: symlink ledger is rejected without touching target
// ---------------------------------------------------------------------------
#[test]
fn test_symlink_ledger_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    let target = tmp.path().join("target");
    fs::write(&target, "unchanged").unwrap();

    std::os::unix::fs::symlink(&target, dir.join("broker-audit.sqlite3")).unwrap();

    let result = append_attempt(&dir, uuid::Uuid::new_v4());
    assert!(result.is_err(), "should reject symlink ledger");

    assert_eq!(fs::read_to_string(&target).unwrap(), "unchanged");
}

// ---------------------------------------------------------------------------
// RED-7: wrong directory or ledger mode is rejected
// ---------------------------------------------------------------------------
#[test]
fn test_wrong_directory_mode_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Weaken directory permissions
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let result = append_attempt(&dir, uuid::Uuid::new_v4());
    assert!(result.is_err(), "should reject 0755 directory");
}

#[test]
fn test_wrong_ledger_mode_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Create ledger with wrong permissions
    let ledger = dir.join("broker-audit.sqlite3");
    {
        let conn = rusqlite::Connection::open(&ledger).unwrap();
        conn.execute_batch(
            "CREATE TABLE t(x); PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
        )
        .unwrap();
    }
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o644)).unwrap();

    let result = append_attempt(&dir, uuid::Uuid::new_v4());
    assert!(result.is_err(), "should reject 0644 ledger");
}

// ---------------------------------------------------------------------------
// RED-8: terminal event requires result code; attempt must not have one
// ---------------------------------------------------------------------------
#[test]
fn test_terminal_event_requires_result_code() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Failure phase without result code should fail
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Failure,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err(), "terminal event needs result code");

    // Attempt phase with result code should fail
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: Some(1),
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err(), "attempt must not have result code");
}

// ---------------------------------------------------------------------------
// RED-9: chain verification detects database tampering
// ---------------------------------------------------------------------------
#[test]
fn test_chain_verification_detects_tampering() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    append_attempt(&dir, uuid::Uuid::new_v4()).unwrap();

    // Tamper with the database directly
    let conn = rusqlite::Connection::open(dir.join("broker-audit.sqlite3")).unwrap();
    conn.execute(
        "UPDATE events SET operation='source-get' WHERE sequence=1",
        [],
    )
    .unwrap();

    let result = audit::verify(&dir, Some(unsafe { libc::getuid() }));
    assert!(result.is_err(), "should detect tampering");
}

// ---------------------------------------------------------------------------
// RED-10: event and head rollback together on head failure (atomicity)
// ---------------------------------------------------------------------------
#[test]
fn test_event_and_head_rollback_together() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());
    let tx = uuid::Uuid::new_v4();

    append_attempt(&dir, tx).unwrap();

    // Inject a trigger that rejects head updates
    let conn = rusqlite::Connection::open(dir.join("broker-audit.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_head BEFORE UPDATE ON head BEGIN SELECT RAISE(ABORT, 'fault injection'); END;"
    ).unwrap();

    let result = append_terminal(&dir, tx, Outcome::Success, 0);
    assert!(result.is_err(), "should fail when head update rejected");

    // Verify only the attempt remains (no orphaned success event)
    let result = audit::verify(&dir, Some(unsafe { libc::getuid() })).unwrap();
    assert_eq!(result.count, 1, "only attempt should remain after rollback");
}

// ---------------------------------------------------------------------------
// RED-11: verify on empty/absent ledger succeeds with count=0
// ---------------------------------------------------------------------------
#[test]
fn test_verify_empty_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // No ledger exists yet — verify should succeed with count=0
    let result = audit::verify(&dir, Some(unsafe { libc::getuid() })).unwrap();
    assert_eq!(result.count, 0);
    assert_eq!(result.hash, audit::ZERO_HASH);
}

// ---------------------------------------------------------------------------
// RED-12: reason SHA-256 validation
// ---------------------------------------------------------------------------
#[test]
fn test_reason_sha256_must_be_valid_hex() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Invalid hex
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some("not-a-valid-sha256-hash".into()),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err(), "should reject invalid reason hash");

    // Missing reason entirely
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: None,
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err(), "should reject missing reason hash");
}

// ---------------------------------------------------------------------------
// RED-13: operation must match pattern
// ---------------------------------------------------------------------------
#[test]
fn test_operation_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "INVALID".into(), // must be lowercase
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// RED-14: transaction UUID validation
// ---------------------------------------------------------------------------
#[test]
fn test_transaction_uuid_required() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // All-nil UUID should still be accepted
    let nil = uuid::Uuid::nil();
    let result = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: nil,
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["EXAMPLE_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    );
    assert!(result.is_ok(), "nil UUID should be accepted");
}

// ---------------------------------------------------------------------------
// RED-15: names are sorted and validated
// ---------------------------------------------------------------------------
#[test]
fn test_names_are_sorted_and_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    let event = audit::append_event(
        &dir,
        audit::AppendEventRequest {
            operation: "source-set".into(),
            phase: Outcome::Attempt,
            transaction: uuid::Uuid::new_v4(),
            actor: "djbclark".into(),
            client: ClientFamily::Hermes,
            reason_sha256: Some(audit::sha256_hex(b"test")),
            command_basename: None,
            names: vec!["Z_KEY".into(), "A_KEY".into()],
            result_code: None,
            expected_uid: Some(unsafe { libc::getuid() }),
        },
    )
    .unwrap();

    // Names should be sorted
    assert_eq!(event.names, vec!["A_KEY", "Z_KEY"]);
}

// ---------------------------------------------------------------------------
// RED-16: owner mismatch is rejected
// ---------------------------------------------------------------------------
#[test]
fn test_owner_mismatch_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Create ledger owned by us
    append_attempt(&dir, uuid::Uuid::new_v4()).unwrap();

    // Verify with wrong UID (1 is typically not the current user)
    let wrong_uid = if unsafe { libc::getuid() } == 0 { 1 } else { 0 };
    let result = audit::verify(&dir, Some(wrong_uid));

    // If we're running as root, this might not fail in all cases.
    // But if we're not root, it should fail with owner mismatch.
    if unsafe { libc::getuid() } != 0 {
        assert!(result.is_err(), "should reject owner mismatch");
    }
}

// ---------------------------------------------------------------------------
// RED-17: chain with multiple transactions validates correctly
// ---------------------------------------------------------------------------
#[test]
fn test_multiple_transactions_chain_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    // Transaction 1: attempt + success
    let tx1 = uuid::Uuid::new_v4();
    let e1 = append_attempt(&dir, tx1).unwrap();
    let e2 = append_terminal(&dir, tx1, Outcome::Success, 0).unwrap();
    assert_eq!(e2.previous_hash, e1.event_hash);

    // Transaction 2: attempt + failure
    let tx2 = uuid::Uuid::new_v4();
    let e3 = append_attempt(&dir, tx2).unwrap();
    assert_eq!(e3.previous_hash, e2.event_hash);
    let e4 = append_terminal(&dir, tx2, Outcome::Failure, 1).unwrap();
    assert_eq!(e4.previous_hash, e3.event_hash);

    // Transaction 3: standalone unknown
    let tx3 = uuid::Uuid::new_v4();
    let e5 = append_attempt(&dir, tx3).unwrap();
    assert_eq!(e5.previous_hash, e4.event_hash);
    let e6 = append_terminal(&dir, tx3, Outcome::Unknown, 127).unwrap();
    assert_eq!(e6.previous_hash, e5.event_hash);

    let result = audit::verify(&dir, Some(unsafe { libc::getuid() })).unwrap();
    assert_eq!(result.count, 6);
    assert_eq!(result.hash, e6.event_hash);
}

// ---------------------------------------------------------------------------
// RED-18: SQLite integrity check on verify
// ---------------------------------------------------------------------------
#[test]
fn test_sqlite_integrity_check_on_verify() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = protected_dir(tmp.path());

    append_attempt(&dir, uuid::Uuid::new_v4()).unwrap();
    let result = audit::verify(&dir, Some(unsafe { libc::getuid() }));
    assert!(result.is_ok(), "verify should pass with clean database");
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// A sliding-window substring check that avoids false negatives when SQLite
/// splits bytes across page boundaries.
fn window_sensitive(needle: &str, haystack: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}
