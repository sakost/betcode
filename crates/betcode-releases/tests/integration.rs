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

/// Send a request to the app and return (status, body text).
async fn send_request(uri: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri);
    for &(name, value) in headers {
        builder = builder.header(name, value);
    }
    let resp = app()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Send a request and assert the response is a redirect containing `expected_asset`.
async fn assert_redirect(uri: &str, user_agent: &str, expected_asset: &str) {
    let mut builder = Request::builder().uri(uri);
    builder = builder.header("user-agent", user_agent);
    let resp = app()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains(expected_asset), "location: {location}");
}

#[tokio::test]
async fn root_browser_returns_html() {
    let (status, text) = send_request("/", &[("accept", "text/html")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("<!DOCTYPE html>"), "should return HTML page");
    assert!(text.contains("BetCode"), "should contain project name");
}

#[tokio::test]
async fn root_curl_returns_shell_script() {
    let (status, text) = send_request("/", &[("user-agent", "curl/8.0")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("#!/bin/sh"), "should return shell script");
    assert!(text.contains("sakost/betcode"), "should have repo replaced");
}

#[tokio::test]
async fn install_sh_returns_script() {
    let (status, text) = send_request("/install.sh", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("#!/bin/sh"));
}

#[tokio::test]
async fn install_ps1_returns_script() {
    let (status, text) = send_request("/install.ps1", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("Requires -Version"));
}

#[tokio::test]
async fn binary_redirect_linux() {
    assert_redirect(
        "/betcode",
        "curl/8.0 (x86_64-linux-gnu)",
        "betcode-linux-amd64.tar.gz",
    )
    .await;
}

#[tokio::test]
async fn binary_redirect_macos_arm64() {
    assert_redirect(
        "/betcode",
        "curl/8.4.0 (aarch64-apple-darwin23.0)",
        "betcode-darwin-arm64.tar.gz",
    )
    .await;
}

#[tokio::test]
async fn unknown_binary_returns_404() {
    let (status, _) = send_request("/unknown-binary", &[("user-agent", "curl/8.0")]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relay_on_macos_returns_404() {
    let (status, _) = send_request(
        "/betcode-relay",
        &[("user-agent", "curl/8.0 (aarch64-apple-darwin)")],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
