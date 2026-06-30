//! End-to-end trace-context propagation with axum and tower.
//!
//! Shows two scenarios:
//!
//! 1. **Incoming traceparent** — the tower middleware recognises the header,
//!    advances the parent-id to create a child span, and stores the result in
//!    request extensions. The axum extractor picks it up from there.
//!
//! 2. **No traceparent** — the middleware starts a fresh root span with random
//!    IDs. The handler still receives a valid `TraceContext`.
//!
//! Run with:
//!   cargo run --example propagation -p trace-context-level3-axum

use axum::Router;
use axum::body::Body;
use axum::routing::get;
use http::Request;
use tower::ServiceExt as _;
use trace_context_level3_axum::TraceContext;
use trace_context_level3_http::TRACEPARENT;
use trace_context_level3_tower::TraceContextLayer;

/// Handler: returns the current traceparent as plain text.
///
/// When the tower middleware is active the extractor reads the already-advanced
/// child span from request extensions rather than re-parsing the raw header.
async fn show_traceparent(ctx: TraceContext) -> String {
    format!(
        "version={:02x} trace-id={} parent-id={} sampled={}\n",
        ctx.traceparent.version,
        ctx.traceparent.trace_id,
        ctx.traceparent.parent_id,
        ctx.traceparent.is_sampled(),
    )
}

fn app() -> Router {
    Router::new()
        .route("/", get(show_traceparent))
        .layer(TraceContextLayer::new())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // ── Scenario 1: incoming traceparent ─────────────────────────────────────
    //
    // The middleware reads the parent's traceparent, generates a new parent-id,
    // and stores the child span in extensions. The trace-id is preserved.

    let incoming = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    let req = Request::builder()
        .uri("/")
        .header(TRACEPARENT, incoming)
        .body(Body::empty())
        .unwrap();

    let resp = app().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let child_info = String::from_utf8(body.to_vec()).unwrap();

    println!("── Scenario 1: incoming traceparent ──────────────────────────");
    println!("incoming:  {incoming}");
    println!("handler:   {child_info}");

    // The trace-id must be preserved; the parent-id must be fresh.
    assert!(
        child_info.contains("trace-id=4bf92f3577b34da6a3ce929d0e0e4736"),
        "trace-id must be forwarded unchanged"
    );
    assert!(
        !child_info.contains("parent-id=00f067aa0ba902b7"),
        "parent-id must be advanced to a new value"
    );

    // ── Scenario 2: no incoming traceparent ───────────────────────────────────
    //
    // The middleware finds no traceparent header and starts a brand-new root
    // span with freshly generated random IDs.

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let resp = app().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let root_info = String::from_utf8(body.to_vec()).unwrap();

    println!("── Scenario 2: no incoming traceparent ───────────────────────");
    println!("handler:   {root_info}");

    assert!(
        !root_info.contains("trace-id=00000000000000000000000000000000"),
        "fresh trace-id must not be all zeros"
    );
}
