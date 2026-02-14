//! Shared database types and utilities.
//!
//! Provides `DatabaseError`, `unix_timestamp()`, pool creation helpers,
//! and base64 encode/decode used by both betcode-daemon and betcode-relay
//! storage layers.

use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use tracing::info;

/// Database errors shared across daemon and relay.
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
        Self::Query(e.to_string())
    }
}

/// Open (or create) a `SQLite` connection pool at the given file path.
///
/// Creates the parent directory if it does not exist, enables WAL journal
/// mode, foreign keys, and sets a 5-second busy timeout.
pub async fn open_pool(path: &Path) -> Result<Pool<Sqlite>, DatabaseError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DatabaseError::Io(e.to_string()))?;
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", path.display()))
        .map_err(|e| DatabaseError::Connection(e.to_string()))?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| DatabaseError::Connection(e.to_string()))?;

    info!(path = %path.display(), "Database opened");

    Ok(pool)
}

/// Open an in-memory `SQLite` connection pool (for testing).
pub async fn open_pool_in_memory() -> Result<Pool<Sqlite>, DatabaseError> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(|e| DatabaseError::Connection(e.to_string()))?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| DatabaseError::Connection(e.to_string()))?;

    Ok(pool)
}

/// Returns the current time as a Unix timestamp (seconds since epoch).
#[allow(clippy::cast_possible_wrap)]
pub fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Macro to define a `Database`-like struct with `open`, `open_in_memory`,
/// `run_migrations`, and `pool` methods.
///
/// Usage:
/// ```ignore
/// betcode_core::define_database!(Database, "Database migrations complete");
/// betcode_core::define_database!(RelayDatabase, "Relay database migrations complete");
/// ```
///
/// The generated struct has:
/// - `pub async fn open(path: &Path) -> Result<Self, DatabaseError>`
/// - `pub async fn open_in_memory() -> Result<Self, DatabaseError>`
/// - `async fn run_migrations(&self) -> Result<(), DatabaseError>`
/// - `pub const fn pool(&self) -> &Pool<Sqlite>`
#[macro_export]
macro_rules! define_database {
    ($name:ident, $migration_msg:expr) => {
        #[derive(Clone)]
        pub struct $name {
            pool: ::sqlx::Pool<::sqlx::Sqlite>,
        }

        impl $name {
            /// Open or create a database at the given path.
            pub async fn open(
                path: &::std::path::Path,
            ) -> ::std::result::Result<Self, $crate::db::DatabaseError> {
                let pool = $crate::db::open_pool(path).await?;
                let db = Self { pool };
                db.run_migrations().await?;
                Ok(db)
            }

            /// Open an in-memory database (for testing).
            pub async fn open_in_memory() -> ::std::result::Result<Self, $crate::db::DatabaseError>
            {
                let pool = $crate::db::open_pool_in_memory().await?;
                let db = Self { pool };
                db.run_migrations().await?;
                Ok(db)
            }

            /// Run database migrations.
            async fn run_migrations(&self) -> ::std::result::Result<(), $crate::db::DatabaseError> {
                ::sqlx::migrate!("./migrations")
                    .run(&self.pool)
                    .await
                    .map_err(|e| $crate::db::DatabaseError::Migration(e.to_string()))?;

                ::tracing::info!($migration_msg);
                Ok(())
            }

            /// Get a reference to the connection pool.
            pub const fn pool(&self) -> &::sqlx::Pool<::sqlx::Sqlite> {
                &self.pool
            }
        }
    };
}

/// Simple base64 encoding (no external dependency needed).
pub fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;

        let _ = result.write_char(CHARS[(n >> 18 & 0x3F) as usize] as char);
        let _ = result.write_char(CHARS[(n >> 12 & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            let _ = result.write_char(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            let _ = result.write_char(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// Simple base64 decoding for stored event payloads.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    #[allow(clippy::cast_possible_truncation)]
    const DECODE: [u8; 128] = {
        let mut table = [255u8; 128];
        let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < 64 {
            table[chars[i] as usize] = i as u8;
            i += 1;
        }
        table
    };

    let input = input.trim_end_matches('=');
    if input.len() % 4 == 1 {
        return Err("Invalid base64 length".to_string());
    }
    let mut result = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.as_bytes().chunks(4) {
        let mut n: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b as usize >= 128 || DECODE[b as usize] == 255 {
                return Err(format!("Invalid base64 character: {}", b as char));
            }
            n |= u32::from(DECODE[b as usize]) << (18 - i * 6);
        }

        result.push((n >> 16 & 0xFF) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8 & 0xFF) as u8);
        }
        if chunk.len() > 3 {
            result.push((n & 0xFF) as u8);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_base64() {
        let data = b"Hello, BetCode!";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn base64_empty() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn base64_padding() {
        // 1 byte → 4 chars with == padding
        let encoded = base64_encode(b"A");
        assert!(encoded.ends_with("=="));
        assert_eq!(base64_decode(&encoded).unwrap(), b"A");

        // 2 bytes → 4 chars with = padding
        let encoded = base64_encode(b"AB");
        assert!(encoded.ends_with('='));
        assert_eq!(base64_decode(&encoded).unwrap(), b"AB");
    }

    #[test]
    fn unix_timestamp_is_reasonable() {
        let ts = unix_timestamp();
        // Should be after 2024-01-01
        assert!(ts > 1_704_067_200);
    }
}
