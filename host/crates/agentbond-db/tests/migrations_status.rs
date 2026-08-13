use agentbond_db::Db;
use agentbond_db::test_util::pg_test_lock;

fn database_url() -> String {
    std::env::var("AGENTBOND_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond".into()
    })
}

async fn setup() -> (std::fs::File, Db) {
    let lock = pg_test_lock().expect("pg lock");
    let db = Db::connect(&database_url())
        .await
        .expect("postgres on 5433");
    db.migrate().await.expect("migrate");
    (lock, db)
}

#[tokio::test]
async fn migrations_status_ok_when_current() {
    let (_lock, db) = setup().await;
    db.migrations_status().await.expect("current");
    assert!(db.migrations_current().await.expect("bool"));
}

#[tokio::test]
async fn migrations_status_rejects_unknown_applied_version() {
    let (_lock, db) = setup().await;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
         VALUES (999999, 'ghost', NOW(), true, decode('00', 'hex'), 1)
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(db.pool())
    .await
    .expect("insert ghost");

    let err = db
        .migrations_status()
        .await
        .expect_err("unknown applied version");
    assert!(
        err.to_string().contains("unknown applied") || err.to_string().contains("999999"),
        "{err}"
    );

    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 999999")
        .execute(db.pool())
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn migrations_status_rejects_failed_and_checksum_mismatch() {
    let (_lock, db) = setup().await;

    let (version, checksum): (i64, Vec<u8>) =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version LIMIT 1")
            .fetch_one(db.pool())
            .await
            .expect("row");

    sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = $1")
        .bind(version)
        .execute(db.pool())
        .await
        .expect("mark failed");
    let err = db.migrations_status().await.expect_err("failed");
    assert!(err.to_string().contains("failed"), "{err}");

    sqlx::query(
        "UPDATE _sqlx_migrations SET success = true, checksum = decode('ff', 'hex') WHERE version = $1",
    )
    .bind(version)
    .execute(db.pool())
    .await
    .expect("bad checksum");
    let err = db.migrations_status().await.expect_err("checksum");
    assert!(err.to_string().contains("checksum"), "{err}");

    sqlx::query("UPDATE _sqlx_migrations SET success = true, checksum = $2 WHERE version = $1")
        .bind(version)
        .bind(&checksum)
        .execute(db.pool())
        .await
        .expect("restore");
    db.migrations_status().await.expect("restored");
}
