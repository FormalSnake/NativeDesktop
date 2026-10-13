mod gate_m7_data {
    use futures_executor::block_on;
    use nd_data::{Database, Error, Migration, OpenOptions, SqliteExecutor, Statement, StepResult, Value, params};
    use serde::Deserialize;

    const MIGRATIONS: &[Migration] = &[
        Migration { version: 1, sql: "CREATE TABLE note (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body BLOB)" },
        Migration { version: 2, sql: "ALTER TABLE note ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0" },
    ];

    #[derive(Debug, Deserialize, PartialEq)]
    struct Note {
        id: i64,
        title: String,
        pinned: i64,
    }

    async fn db() -> Database {
        let db = Database::open_memory().await.unwrap();
        assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 2);
        db
    }

    #[test]
    fn query_mutate_and_typed_rows() {
        block_on(async {
            let db = db().await;
            let run =
                db.mutate("INSERT INTO note (title, body) VALUES (?, ?)", params!["a", vec![1u8, 2]]).await.unwrap();
            assert_eq!((run.changes, run.last_insert_rowid), (1, 1));
            db.mutate("INSERT INTO note (title, pinned) VALUES (?, ?)", params!["b", true]).await.unwrap();

            let rows = db.query("SELECT * FROM note ORDER BY id", vec![]).await.unwrap();
            assert_eq!(rows[0].get("body"), Some(&Value::Blob(vec![1, 2])));
            assert_eq!(rows[1].get("pinned"), Some(&Value::Integer(1)));

            let notes: Vec<Note> =
                db.query_as("SELECT id, title, pinned FROM note WHERE pinned = ?", params![1]).await.unwrap();
            assert_eq!(notes, [Note { id: 2, title: "b".into(), pinned: 1 }]);
        });
    }

    #[test]
    fn transaction_is_all_or_nothing() {
        block_on(async {
            let db = db().await;
            let bad = vec![
                Statement::new("INSERT INTO note (title) VALUES (?)", params!["kept?"]),
                Statement::new("INSERT INTO note (title) VALUES (?)", params![None::<String>]),
            ];
            assert!(matches!(db.transaction(bad).await, Err(Error::Sqlite(_))));
            assert!(db.query("SELECT 1 FROM note", vec![]).await.unwrap().is_empty());

            let good = vec![
                Statement::new("INSERT INTO note (title) VALUES (?)", params!["x"]),
                Statement::new("SELECT count(*) AS n FROM note", vec![]),
            ];
            let out = db.transaction(good).await.unwrap();
            let StepResult::Rows(rows) = &out[1] else { panic!("{out:?}") };
            assert_eq!(rows[0].get("n"), Some(&Value::Integer(1)));
        });
    }

    #[test]
    fn migrations_resume_and_refuse_a_newer_schema() {
        block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested/app.db");
            let first = Database::open(OpenOptions::file(&path)).await.unwrap();
            assert_eq!(first.migrate(&MIGRATIONS[..1]).await.unwrap(), 1);
            first.close().await.unwrap();

            let again = Database::open(OpenOptions::file(&path)).await.unwrap();
            assert_eq!(again.migrate(MIGRATIONS).await.unwrap(), 2);
            assert_eq!(again.migrate(MIGRATIONS).await.unwrap(), 2);
            assert!(matches!(again.migrate(&MIGRATIONS[..1]).await, Err(Error::NewerSchema { found: 2, known: 1 })));
        });
    }

    #[test]
    fn closed_handles_fail_and_panics_are_contained() {
        block_on(async {
            let db = db().await;
            let other = db.clone();
            let err = db.with(|_| -> nd_data::Result<()> { panic!("boom") }).await.unwrap_err();
            assert!(matches!(&err, Error::Panicked(m) if m == "boom"));
            assert!(other.query("SELECT 1", vec![]).await.is_ok());

            db.close().await.unwrap();
            assert!(matches!(other.query("SELECT 1", vec![]).await, Err(Error::Closed)));
        });
    }

    #[test]
    fn calls_run_off_the_calling_thread() {
        block_on(async {
            let db = db().await;
            let here = std::thread::current().id();
            let there = db.with(|_| Ok(std::thread::current().id())).await.unwrap();
            assert_ne!(here, there);
        });
    }
}
