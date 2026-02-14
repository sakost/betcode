//! Database connection and initialization.

pub use betcode_core::db::DatabaseError;

betcode_core::define_database!(Database, "Database migrations complete");

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_in_memory_works() {
        let db = Database::open_in_memory().await;
        assert!(db.is_ok());
    }
}
