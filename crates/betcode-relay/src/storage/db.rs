//! SQLite database for BetCode relay server.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::str::FromStr;
use tracing::info;

#[derive(Clone)]
pub struct RelayDatabase {
    pool: Pool<Sqlite>,
}

impl RelayDatabase {
    pub async fn open(path: &Path) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DatabaseError::Io(e.to_string()))?;
        }

        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", path.display()))
                .map_err(|e| DatabaseError::Connection(e.to_string()))?
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .foreign_keys(true)
                .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;

        info!(path = %path.display(), "Relay database opened");

        let db = Self { pool };
        db.run_migrations().await?;

        Ok(db)
    }

    pub async fn open_in_memory() -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| DatabaseError::Connection(e.to_string()))?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;

        let db = Self { pool };
        db.run_migrations().await?;

        Ok(db)
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;

        info!("Relay database migrations complete");
        Ok(())
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Query error: {0}")]
    Query(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

impl From<sqlx::Error> for DatabaseError {
    fn from(e: sqlx::Error) -> Self {
        DatabaseError::Query(e.to_string())
    }
}
