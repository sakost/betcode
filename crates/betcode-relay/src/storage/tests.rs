//! Storage layer tests for `BetCode` relay.

use super::db::RelayDatabase;
use super::queries_buffer::{BufferMessageParams, CertificateParams};
use betcode_core::db::unix_timestamp;

async fn test_db() -> RelayDatabase {
    RelayDatabase::open_in_memory().await.unwrap()
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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

    let user = db.get_user_by_username("alice").await.unwrap();
    assert_eq!(user.id, "u1");

    assert!(db.get_user_by_username("bob").await.is_err());
}

// === Token tests ===

#[tokio::test]
async fn create_and_get_token() {
    let db = test_db().await;
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "tokenhash", future)
        .await
        .unwrap();

    let found = db.get_token_by_hash("tokenhash").await.unwrap();
    assert!(found.is_some());

    db.create_token("t2", "u1", "expiredhash", unix_timestamp() - 1)
        .await
        .unwrap();
    let not_found = db.get_token_by_hash("expiredhash").await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn revoke_token() {
    let db = test_db().await;
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

    let future = unix_timestamp() + 3600;
    db.create_token("t1", "u1", "tokenhash", future)
        .await
        .unwrap();

    assert!(db.revoke_token("t1").await.unwrap());

    let found = db.get_token_by_hash("tokenhash").await.unwrap();
    assert!(found.is_none());
}

// === Machine tests ===

#[tokio::test]
async fn create_and_get_machine() {
    let db = test_db().await;
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();
    db.create_machine("m2", "desktop", "u1", "{}")
        .await
        .unwrap();
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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

    assert!(db.remove_machine("m1").await.unwrap());
    assert!(!db.remove_machine("m1").await.unwrap());
    assert!(db.get_machine("m1").await.is_err());
}

// === Machine identity pubkey tests ===

#[tokio::test]
async fn update_and_get_machine_identity_pubkey() {
    let db = test_db().await;
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

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
    db.create_user("u1", "alice", "alice@example.com", "hash123")
        .await
        .unwrap();
    db.create_machine("m1", "laptop", "u1", "{}").await.unwrap();

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
