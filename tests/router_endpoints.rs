use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use tower::util::ServiceExt;

use system_monitor_web::{build_router, AppContext, EMBEDDED_INDEX_HTML};

fn app() -> axum::Router {
    build_router(Arc::new(AppContext::default()))
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("response body should be readable");
    String::from_utf8(bytes.to_vec()).expect("response should be valid utf8")
}

#[tokio::test]
async fn serves_embedded_index_at_root() {
    let response = app()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.starts_with("text/html"));

    let body = response_text(response).await;
    assert_eq!(body, EMBEDDED_INDEX_HTML);
}

#[tokio::test]
async fn serves_embedded_index_at_index_html() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/index.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert_eq!(body, EMBEDDED_INDEX_HTML);
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/not-a-real-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_text(response).await;
    assert_eq!(body, "Not found");
}

#[tokio::test]
async fn refresh_returns_json_with_expected_shape() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/refresh")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.starts_with("application/json"));

    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store, no-cache, must-revalidate, proxy-revalidate")
    );
    assert_eq!(
        response
            .headers()
            .get(header::PRAGMA)
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    assert_eq!(
        response
            .headers()
            .get(header::EXPIRES)
            .and_then(|v| v.to_str().ok()),
        Some("0")
    );

    let body = response_text(response).await;
    let json: Value = serde_json::from_str(&body).expect("refresh payload should be valid JSON");

    for key in [
        "cpu",
        "cpu_model",
        "cores",
        "mem",
        "gpu",
        "network",
        "temperatures",
        "top_cpu",
        "top_mem",
        "top_net",
        "uptime",
        "loadavg",
    ] {
        assert!(json.get(key).is_some(), "missing key: {key}");
    }
}

#[tokio::test]
async fn stat_endpoint_returns_text_plain_with_proc_stat_content() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/system/stat")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.starts_with("text/plain"));

    let body = response_text(response).await;
    assert!(body.contains("cpu"), "expected /proc/stat style content");
}
