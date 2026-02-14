use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use betcode_releases::routes::{build_router, AppState};

fn app() -> axum::Router {
    build_router(AppState {
        repo: "sakost/betcode".to_string(),
        base_url: "get.betcode.dev".to_string(),
    })
}

#[tokio::test]
async fn root_browser_returns_html() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("<!DOCTYPE html>"), "should return HTML page");
    assert!(text.contains("BetCode"), "should contain project name");
}

#[tokio::test]
async fn root_curl_returns_shell_script() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("user-agent", "curl/8.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("#!/bin/sh"), "should return shell script");
    assert!(text.contains("sakost/betcode"), "should have repo replaced");
}

#[tokio::test]
async fn install_sh_returns_script() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/install.sh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("#!/bin/sh"));
}

#[tokio::test]
async fn install_ps1_returns_script() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/install.ps1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("Requires -Version"));
}

#[tokio::test]
async fn binary_redirect_linux() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/betcode")
                .header("user-agent", "curl/8.0 (x86_64-linux-gnu)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(
        location.contains("betcode-linux-amd64.tar.gz"),
        "location: {location}"
    );
}

#[tokio::test]
async fn binary_redirect_macos_arm64() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/betcode")
                .header("user-agent", "curl/8.4.0 (aarch64-apple-darwin23.0)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(
        location.contains("betcode-darwin-arm64.tar.gz"),
        "location: {location}"
    );
}

#[tokio::test]
async fn unknown_binary_returns_404() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/unknown-binary")
                .header("user-agent", "curl/8.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relay_on_macos_returns_404() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/betcode-relay")
                .header("user-agent", "curl/8.0 (aarch64-apple-darwin)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
