//! Tests for machine ownership verification in `TunnelService`.

use std::sync::Arc;

use tonic::{Code, Request};

use betcode_proto::v1::TunnelHeartbeat;
use betcode_proto::v1::tunnel_service_server::TunnelService;

use crate::buffer::BufferManager;
use crate::registry::ConnectionRegistry;
use crate::server::test_helpers::{
    test_claims, test_claims_u2, test_db_with_owner, test_db_with_two_users,
};
use crate::server::tunnel_svc::TunnelServiceImpl;
use crate::storage::RelayDatabase;

/// Build a `TunnelServiceImpl` from the given database.
fn build_service(db: RelayDatabase) -> TunnelServiceImpl {
    let registry = Arc::new(ConnectionRegistry::new());
    let buffer = Arc::new(BufferManager::new(db.clone(), Arc::clone(&registry)));
    TunnelServiceImpl::new(registry, db, buffer)
}

/// Build a `TunnelServiceImpl` backed by an in-memory DB that already
/// contains user "u1" (alice) and machine "m1" owned by "u1".
async fn setup() -> TunnelServiceImpl {
    build_service(test_db_with_owner().await)
}

/// Build a `TunnelServiceImpl` backed by an in-memory DB that also contains
/// a second user "u2" (eve) who does NOT own machine "m1".
async fn setup_with_second_user() -> TunnelServiceImpl {
    build_service(test_db_with_two_users().await)
}

/// Attach `test_claims()` (sub = "u1") to a request.
fn attach_claims(req: &mut Request<TunnelHeartbeat>) {
    req.extensions_mut().insert(test_claims());
}

/// Attach claims for user "u2" (eve) to a request.
fn attach_wrong_owner_claims(req: &mut Request<TunnelHeartbeat>) {
    req.extensions_mut().insert(test_claims_u2());
}

// ── heartbeat: ownership check ──────────────────────────────────────

#[tokio::test]
async fn heartbeat_owner_succeeds() {
    let svc = setup().await;

    let mut req = Request::new(TunnelHeartbeat {
        machine_id: "m1".to_string(),
        ..Default::default()
    });
    attach_claims(&mut req);

    let resp = svc.heartbeat(req).await.unwrap();
    assert_eq!(resp.into_inner().machine_id, "m1");
}

#[tokio::test]
async fn heartbeat_non_owner_gets_permission_denied() {
    let svc = setup_with_second_user().await;

    let mut req = Request::new(TunnelHeartbeat {
        machine_id: "m1".to_string(),
        ..Default::default()
    });
    attach_wrong_owner_claims(&mut req);

    let err = svc.heartbeat(req).await.unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert!(
        err.message().contains("Not your machine"),
        "expected 'Not your machine', got: {}",
        err.message()
    );
}

#[tokio::test]
async fn heartbeat_nonexistent_machine_gets_not_found() {
    let svc = setup().await;

    let mut req = Request::new(TunnelHeartbeat {
        machine_id: "no-such-machine".to_string(),
        ..Default::default()
    });
    attach_claims(&mut req);

    let err = svc.heartbeat(req).await.unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert!(
        err.message().contains("Machine not found"),
        "expected 'Machine not found', got: {}",
        err.message()
    );
}
