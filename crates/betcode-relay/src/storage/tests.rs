//! Storage layer tests for `BetCode` relay.

use super::db::RelayDatabase;
use super::queries_buffer::{BufferMessageParams, CertificateParams};
use betcode_core::db::unix_timestamp;

async fn test_db() -> RelayDatabase {
    RelayDatabase::open_in_memory().await.unwrap()
}

/// Create the default test user ("u1", "alice") in the database.
async fn create_test_user(db: &RelayDatabase) {
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
}

/// Create a test machine owned by user "u1".
async fn create_test_machine(db: &RelayDatabase, machine_id: &str, name: &str) {
    db.create_machine(machine_id, name, "u1", "{}")
        .await
        .unwrap();
}

/// Create the default test user and a single machine ("m1", "laptop").
async fn setup_user_and_machine(db: &RelayDatabase) {
    create_test_user(db).await;
    create_test_machine(db, "m1", "laptop").await;
}

// === User tests ===

#[tokio::test]
async fn create_and_get_user() {
    let db = test_db().await;
    let user = db
        .create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

    assert_eq!(user.id, "u1");
    assert_eq!(user.username, "alice");
    assert_eq!(user.email, "alice@example.com");
}

#[tokio::test]
async fn get_user_by_username() {
    let db = test_db().await;
    create_test_user(&db).await;

    let user = db.get_user_by_username("alice").await.unwrap();
    assert_eq!(user.id, "u1");

    assert!(db.get_user_by_username("bob").await.is_err());
}

// === Token tests ===

#[tokio::test]
async fn create_and_get_token() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    let token = db
        .create_token("t1", "u1", "tokenhash", future)
        .await
        .unwrap();

    assert_eq!(token.id, "t1");
    assert_eq!(token.user_id, "u1");
    assert_eq!(token.revoked, 0);
}

#[tokio::test]
async fn find_token_by_hash() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "tokenhash", future)
        .await
        .unwrap();

    let found = db.get_token_by_hash("tokenhash", 0).await.unwrap();
    assert!(found.is_some());

    db.create_token("t2", "u1", "expiredhash", unix_timestamp() - 1)
        .await
        .unwrap();
    let not_found = db.get_token_by_hash("expiredhash", 0).await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn revoke_token() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "tokenhash", future)
        .await
        .unwrap();

    assert!(db.revoke_token("t1").await.unwrap());

    let found = db.get_token_by_hash("tokenhash", 0).await.unwrap();
    assert!(found.is_none());
}

// === Token rotation tests ===

#[tokio::test]
async fn rotate_token_sets_rotated_at_and_successor() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();
    db.create_token("t2", "u1", "hash2", future).await.unwrap();

    let now = unix_timestamp();
    assert!(db.rotate_token("t1", "t2").await.unwrap());

    let token = db.get_token("t1").await.unwrap();
    assert!(token.rotated_at.is_some());
    let rotated_at = token.rotated_at.unwrap();
    assert!((rotated_at - now).abs() <= 1);
    assert_eq!(token.successor_id.as_deref(), Some("t2"));
    assert_eq!(token.revoked, 0);
}

#[tokio::test]
async fn rotate_token_preserves_original_rotated_at() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();
    db.create_token("t2", "u1", "hash2", future).await.unwrap();
    db.create_token("t3", "u1", "hash3", future).await.unwrap();

    // First rotation
    assert!(db.rotate_token("t1", "t2").await.unwrap());
    let first_rotated_at = db.get_token("t1").await.unwrap().rotated_at.unwrap();

    // Small delay is not needed — COALESCE keeps original value regardless.
    // Second rotation with a different successor.
    assert!(db.rotate_token("t1", "t3").await.unwrap());
    let token = db.get_token("t1").await.unwrap();

    // rotated_at is preserved from first call (COALESCE)
    assert_eq!(token.rotated_at.unwrap(), first_rotated_at);
    // successor_id is updated to the latest
    assert_eq!(token.successor_id.as_deref(), Some("t3"));
}

#[tokio::test]
async fn rotate_token_noop_on_revoked_token() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();
    db.create_token("t2", "u1", "hash2", future).await.unwrap();

    // Hard-revoke first
    assert!(db.revoke_token("t1").await.unwrap());

    // rotate_token should be a no-op (WHERE revoked = 0 won't match)
    assert!(!db.rotate_token("t1", "t2").await.unwrap());

    let token = db.get_token("t1").await.unwrap();
    assert_eq!(token.revoked, 1);
    assert!(token.rotated_at.is_none());
    assert!(token.successor_id.is_none());
}

#[tokio::test]
async fn get_token_by_hash_returns_active_token() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();

    let found = db.get_token_by_hash("hash1", 30).await.unwrap();
    assert!(found.is_some());
    let token = found.unwrap();
    assert_eq!(token.id, "t1");
    assert!(token.rotated_at.is_none());
}

#[tokio::test]
async fn get_token_by_hash_returns_recently_rotated() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();
    db.create_token("t2", "u1", "hash2", future).await.unwrap();

    // Rotate t1 → t2 (rotated_at = now)
    db.rotate_token("t1", "t2").await.unwrap();

    // Within grace window of 30s, should still be found
    let found = db.get_token_by_hash("hash1", 30).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "t1");
}

#[tokio::test]
async fn get_token_by_hash_excludes_old_rotated() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();
    db.create_token("t2", "u1", "hash2", future).await.unwrap();

    // Rotate t1, then manually backdate rotated_at to simulate expired grace
    db.rotate_token("t1", "t2").await.unwrap();
    sqlx::query("UPDATE tokens SET rotated_at = ? WHERE id = ?")
        .bind(unix_timestamp() - 60) // 60s ago — past any reasonable grace window
        .bind("t1")
        .execute(db.pool())
        .await
        .unwrap();

    // With a 30s grace window, should NOT be found
    let found = db.get_token_by_hash("hash1", 30).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn get_token_by_hash_excludes_hard_revoked() {
    let db = test_db().await;
    create_test_user(&db).await;

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "hash1", future).await.unwrap();

    db.revoke_token("t1").await.unwrap();

    // Hard-revoked token should not be found even with grace window
    let found = db.get_token_by_hash("hash1", 30).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn get_token_by_hash_excludes_expired() {
    let db = test_db().await;
    create_test_user(&db).await;

    // Token expired 1 second ago
    let past = unix_timestamp() - 1;
    db.create_token("t1", "u1", "hash1", past).await.unwrap();

    let found = db.get_token_by_hash("hash1", 30).await.unwrap();
    assert!(found.is_none());
}

// === Machine tests ===

#[tokio::test]
async fn create_and_get_machine() {
    let db = test_db().await;
    create_test_user(&db).await;

    let machine = db
        .create_machine("m1", "my-laptop", "u1", "{}")
        .await
        .unwrap();

    assert_eq!(machine.id, "m1");
    assert_eq!(machine.name, "my-laptop");
    assert_eq!(machine.owner_id, "u1");
    assert_eq!(machine.status, "offline");
}

#[tokio::test]
async fn list_machines_with_filter() {
    let db = test_db().await;
    create_test_user(&db).await;
    create_test_machine(&db, "m1", "laptop").await;
    create_test_machine(&db, "m2", "desktop").await;
    db.update_machine_status("m1", "online").await.unwrap();

    let all = db.list_machines("u1", None, 100, 0).await.unwrap();
    assert_eq!(all.len(), 2);

    let online = db
        .list_machines("u1", Some("online"), 100, 0)
        .await
        .unwrap();
    assert_eq!(online.len(), 1);

    let offline = db
        .list_machines("u1", Some("offline"), 100, 0)
        .await
        .unwrap();
    assert_eq!(offline.len(), 1);
}

#[tokio::test]
async fn remove_machine() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    assert!(db.remove_machine("m1").await.unwrap());
    assert!(!db.remove_machine("m1").await.unwrap());
    assert!(db.get_machine("m1").await.is_err());
}

// === Machine identity pubkey tests ===

#[tokio::test]
async fn update_and_get_machine_identity_pubkey() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    // Initially no pubkey
    let pubkey = db.get_machine_identity_pubkey("m1").await.unwrap();
    assert!(pubkey.is_none());

    // Set pubkey
    let fake_pubkey = vec![42u8; 32];
    db.update_machine_identity_pubkey("m1", &fake_pubkey)
        .await
        .unwrap();

    // Verify it's stored
    let pubkey = db.get_machine_identity_pubkey("m1").await.unwrap();
    assert_eq!(pubkey.unwrap(), fake_pubkey);

    // Update to a different pubkey
    let new_pubkey = vec![99u8; 32];
    db.update_machine_identity_pubkey("m1", &new_pubkey)
        .await
        .unwrap();
    let pubkey = db.get_machine_identity_pubkey("m1").await.unwrap();
    assert_eq!(pubkey.unwrap(), new_pubkey);
}

// === Buffer tests ===

#[tokio::test]
async fn buffer_and_drain_messages() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    db.buffer_message(&BufferMessageParams {
        machine_id: "m1",
        request_id: "r1",
        method: "Converse",
        payload: b"data1",
        metadata: "{}",
        priority: 0,
        ttl_secs: 3600,
    })
    .await
    .unwrap();
    db.buffer_message(&BufferMessageParams {
        machine_id: "m1",
        request_id: "r2",
        method: "Converse",
        payload: b"data2",
        metadata: "{}",
        priority: 1,
        ttl_secs: 3600,
    })
    .await
    .unwrap();

    let messages = db.drain_buffer("m1").await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].request_id, "r2"); // higher priority
    assert_eq!(messages[1].request_id, "r1");

    // Messages are still in DB until explicitly deleted
    assert_eq!(db.count_buffered_messages("m1").await.unwrap(), 2);

    // Delete each message after "delivery"
    for msg in &messages {
        assert!(db.delete_buffered_message(msg.id).await.unwrap());
    }
    assert_eq!(db.count_buffered_messages("m1").await.unwrap(), 0);
}

#[tokio::test]
async fn cleanup_expired_buffer() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    db.buffer_message(&BufferMessageParams {
        machine_id: "m1",
        request_id: "r1",
        method: "Converse",
        payload: b"old",
        metadata: "{}",
        priority: 0,
        ttl_secs: -1,
    })
    .await
    .unwrap();
    db.buffer_message(&BufferMessageParams {
        machine_id: "m1",
        request_id: "r2",
        method: "Converse",
        payload: b"new",
        metadata: "{}",
        priority: 0,
        ttl_secs: 3600,
    })
    .await
    .unwrap();

    let cleaned = db.cleanup_expired_buffer().await.unwrap();
    assert_eq!(cleaned, 1);
    assert_eq!(db.count_buffered_messages("m1").await.unwrap(), 1);
}

// === Certificate tests ===

#[tokio::test]
async fn create_and_get_certificate() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    let now = unix_timestamp();
    let cert = db
        .create_certificate(&CertificateParams {
            id: "c1",
            machine_id: Some("m1"),
            subject_cn: "m1.betcode.dev",
            serial_number: "SN001",
            not_before: now,
            not_after: now + 86400,
            pem_cert: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----",
        })
        .await
        .unwrap();

    assert_eq!(cert.id, "c1");
    assert_eq!(cert.subject_cn, "m1.betcode.dev");
    assert_eq!(cert.revoked, 0);
}

#[tokio::test]
async fn revoke_certificate_hides_from_machine_certs() {
    let db = test_db().await;
    setup_user_and_machine(&db).await;

    let now = unix_timestamp();
    db.create_certificate(&CertificateParams {
        id: "c1",
        machine_id: Some("m1"),
        subject_cn: "cn",
        serial_number: "SN001",
        not_before: now,
        not_after: now + 86400,
        pem_cert: "pem",
    })
    .await
    .unwrap();

    assert!(db.revoke_certificate("c1").await.unwrap());

    let certs = db.get_machine_certificates("m1").await.unwrap();
    assert!(certs.is_empty());
}
