//! `SQLite` database for `BetCode` relay server.

pub use betcode_core::db::DatabaseError;

betcode_core::define_database!(RelayDatabase, "Relay database migrations complete");
