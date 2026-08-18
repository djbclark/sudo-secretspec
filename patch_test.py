import re
with open("secretspec/src/provider/sqlite.rs", "r") as f:
    content = f.read()

test_code = """
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
        
        provider
            .delete(convention("APP_SECRET"))
            .unwrap();
            
        let entries_after_delete: i64 = conn
            .query_row("SELECT count(*) FROM entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entries_after_delete, 2);
    }
}
"""
content = content.replace("}\n", test_code, 1) # This is dangerous, let's do it better.

