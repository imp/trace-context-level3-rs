//! A working axum web server demonstrating W3C Trace Context propagation.
//!
//! The [`TraceContextLayer`] middleware intercepts every request:
//! - If a valid `traceparent` header arrives it advances the span (new
//!   `parent-id`, same `trace-id`) and stores the result in request extensions.
//! - If the header is absent or malformed it starts a fresh root span.
//!
//! The [`TraceContext`] axum extractor then reads the context from extensions,
//! so the handler always receives an already-advanced child span.
//!
//! # Run
//!
//! ```text
//! cargo run --example server -p trace-context-level3-axum
//! ```
//!
//! # Try it
//!
//! Fresh root span (no incoming header):
//! ```text
//! curl http://127.0.0.1:3000/
//! ```
//!
//! Child span (middleware advances the parent-id, preserves trace-id):
//! ```text
//! curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
//!      http://127.0.0.1:3000/
//! ```
//!
//! With tracestate:
//! ```text
//! curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
//!      -H 'tracestate: vendor=value' \
//!      http://127.0.0.1:3000/
//! ```

use axum::Router;
use axum::routing::get;
use trace_context_level3_axum::TraceContext;
use trace_context_level3_tower::TraceContextLayer;

/// Returns the current trace context as plain text.
///
/// Because [`TraceContextLayer`] runs before this handler, `ctx` contains the
/// child span (advanced `parent-id`), not the raw incoming header value.
async fn show_context(ctx: TraceContext) -> String {
    let tracestate = ctx.tracestate.to_string();
    let tracestate = if tracestate.is_empty() {
        "(none)".to_owned()
    } else {
        tracestate
    };
    format!(
        "traceparent : {tp}\n  version   : {ver:02x}\n  trace-id  : {tid}\n  parent-id : {pid}\n  sampled   : {sampled}\ntracestate  : {ts}\n",
        tp      = ctx.traceparent,
        ver     = ctx.traceparent.version,
        tid     = ctx.traceparent.trace_id,
        pid     = ctx.traceparent.parent_id,
        sampled = ctx.traceparent.is_sampled(),
        ts      = tracestate,
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let app = Router::new()
        .route("/", get(show_context))
        .layer(TraceContextLayer::new());

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    println!("Trace Context example server listening on http://{addr}");
    println!();
    println!("── No incoming traceparent (server starts a fresh root span) ──");
    println!("  curl http://{addr}/");
    println!();
    println!("── With traceparent (server advances the parent-id) ───────────");
    println!("  curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \\");
    println!("       http://{addr}/");
    println!();
    println!("── With traceparent + tracestate ──────────────────────────────");
    println!("  curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \\");
    println!("       -H 'tracestate: vendor=value' \\");
    println!("       http://{addr}/");
    println!();

    axum::serve(listener, app).await.unwrap();
}
