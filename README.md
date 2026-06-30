# trace-context-level3-rs

W3C Trace Context Level 3 implementation in Rust.

Parses and serializes the `traceparent` and `tracestate` HTTP headers, propagates
context through Tower middleware, and exposes an axum extractor — all following the
[W3C Trace Context specification](https://w3c.github.io/trace-context/).

## Workspace

| Crate | Description |
|---|---|
| [`trace-context-level3`] | Core types: `TraceParent`, `TraceState`, `TraceId`, `ParentId`, `TraceFlags` |
| [`trace-context-level3-http`] | HTTP extraction and injection via `TraceContext` |
| [`trace-context-level3-tower`] | Tower middleware (`TraceContextLayer`) and optional task-local storage |
| [`trace-context-level3-axum`] | axum `FromRequestParts` extractor (`TraceContext`) |

---

## Core types — `trace-context-level3`

```toml
[dependencies]
trace-context-level3 = { git = "https://github.com/imp/trace-context-level3-rs" }
```

```rust
use trace_context_level3::{TraceParent, TraceState};

// Parse a traceparent header value
let tp: TraceParent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    .parse()
    .unwrap();

assert_eq!(tp.trace_id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
assert_eq!(tp.parent_id.to_string(), "00f067aa0ba902b7");
assert!(tp.is_sampled());

// Parse and mutate tracestate
let mut state: TraceState = "vendorname=opaquevalue".parse().unwrap();
state.insert("myvendor", "data").unwrap();
assert_eq!(state.to_string(), "myvendor=data,vendorname=opaquevalue");
```

Random ID generation requires the `rand` feature:

```toml
trace-context-level3 = { ..., features = ["rand"] }
```

`serde` support (`Serialize`/`Deserialize` for all public types) is available as an optional feature:

```toml
trace-context-level3 = { ..., features = ["serde"] }
```

Types serialize as their canonical wire-format strings (`TraceParent`, `TraceState`, `TraceId`, `ParentId`) or as a plain `u8` (`TraceFlags`). `TraceContext` in `trace-context-level3-http` (enabled with `trace-context-level3-http/serde`) serializes as a struct with `"traceparent"` and `"tracestate"` string fields:

```json
{
  "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
  "tracestate": "vendor=value"
}
```

---

## HTTP extraction — `trace-context-level3-http`

```toml
[dependencies]
trace-context-level3-http = { git = "https://github.com/imp/trace-context-level3-rs" }
```

```rust
use http::HeaderMap;
use trace_context_level3_http::TraceContext;

// Extract
let mut headers = HeaderMap::new();
headers.insert("traceparent",
    "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".parse().unwrap());

match TraceContext::extract(&headers) {
    Ok(Some(ctx)) => println!("trace-id: {}", ctx.traceparent.trace_id),
    Ok(None)      => println!("no traceparent header"),
    Err(e)        => println!("invalid traceparent: {e}"),
}

// Inject
let mut outgoing = HeaderMap::new();
ctx.inject(&mut outgoing);
```

`extract` returns `Result<Option<Self>, TraceParentError>`:

- `Ok(None)` — no `traceparent` header present
- `Err(_)` — header present but invalid (e.g. multiple values, bad format)
- `Ok(Some(ctx))` — valid context; malformed or missing `tracestate` is treated
  leniently and silently dropped, per spec guidance

---

## Tower middleware — `trace-context-level3-tower`

```toml
[dependencies]
trace-context-level3-tower = { git = "https://github.com/imp/trace-context-level3-rs" }
```

`TraceContextLayer` intercepts every request:

1. If a valid `traceparent` header arrives, it advances the span by generating a
   fresh `parent-id` (preserving the `trace-id`) and stores the result in request
   extensions.
2. If the header is absent or malformed, it starts a new root span with random IDs.

```rust
use tower::ServiceBuilder;
use trace_context_level3_tower::{TraceContextLayer, TraceResponseLayer};

let service = ServiceBuilder::new()
    .layer(TraceContextLayer::new())   // outer: extracts/creates context
    .layer(TraceResponseLayer::new())  // inner: adds Server-Timing to response
    .service(inner);
```

### Response propagation (`Server-Timing`)

`TraceResponseLayer` appends `Server-Timing: trace;desc=<traceparent>` to every
HTTP response, implementing the Level 3 response header. It must be placed after
`TraceContextLayer` in the stack (inside it, closer to the handler).

On the receiving side, `extract_server_timing` reads the header back:

```rust
use trace_context_level3_http::extract_server_timing;

if let Some(tp) = extract_server_timing(response.headers()) {
    println!("server span: {tp}");
}
```

### Task-local storage

Enable the `task-local` feature to additionally store the context in a
`tokio::task_local!` for the duration of each request future, so any code in the
call stack can read it without threading it through function arguments:

```toml
trace-context-level3-tower = { ..., features = ["task-local"] }
```

```rust
use trace_context_level3_tower::{TraceContextLayer, TRACE_CONTEXT};

let layer = TraceContextLayer::new().enable_task_local();

// Inside a handler called from within that layer:
TRACE_CONTEXT.with(|ctx| println!("{}", ctx.traceparent));
```

---

## axum extractors — `trace-context-level3-axum`

```toml
[dependencies]
trace-context-level3-axum = { git = "https://github.com/imp/trace-context-level3-rs" }
```

Two extractors are provided, both implementing `FromRequestParts`:

### `TraceContext` — spec-compliant (recommended)

When `traceparent` is absent or invalid a fresh root span is generated, matching
the W3C spec's guidance ("receivers SHOULD start a new trace"). Handlers always
receive a valid context and the extractor never rejects.

```rust
use axum::{Router, routing::get};
use trace_context_level3_axum::TraceContext;
use trace_context_level3_tower::TraceContextLayer;

async fn handler(ctx: TraceContext) -> String {
    ctx.traceparent.to_string()
}

let app = Router::new()
    .route("/", get(handler))
    .layer(TraceContextLayer::new());
```

### `StrictTraceContext` — policy override

Rejects with `400 Bad Request` when `traceparent` is absent or invalid. Use this
when your service must refuse requests that don't carry an upstream trace.

```rust
use trace_context_level3_axum::StrictTraceContext;

async fn strict_handler(ctx: StrictTraceContext) -> String {
    ctx.traceparent.to_string()
}
```

Use `Option<StrictTraceContext>` to distinguish "context was propagated" from "was
not" without rejecting the request — axum's blanket impl converts any rejection to
`None`.

Both extractors check request extensions first: when `TraceContextLayer` is active
it stores the already-advanced child span there, so neither extractor re-parses
the raw headers.

---

## Running the examples

### In-process propagation demo

Shows both scenarios — incoming traceparent (child span) and no header (root span) —
using `tower::ServiceExt::oneshot` without a real network:

```text
cargo run --example propagation -p trace-context-level3-axum
```

### Live axum server

Starts a server on `127.0.0.1:3000` and prints ready-to-use curl commands:

```text
cargo run --example server -p trace-context-level3-axum
```

```text
# Fresh root span (no incoming header)
curl http://127.0.0.1:3000/

# Child span — middleware advances parent-id, preserves trace-id
curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
     http://127.0.0.1:3000/

# With tracestate
curl -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
     -H 'tracestate: vendor=value' \
     http://127.0.0.1:3000/
```

### Running the full test suite

```text
mise run ci
```

Runs `fmt-check`, `clippy`, `test`, and feature-variant tests in parallel.

---

## Spec compliance

- `traceparent` version `00` parsing and serialisation
- Unknown higher versions normalised to `v00` on parse (forward-compatibility rule)
- `tracestate` entry validation, deduplication (first occurrence wins), and
  truncation to 32 entries using the two-step spec algorithm
- Lenient `tracestate` parsing across multiple header values (comma-separated lists
  are merged; malformed entries are silently dropped)
- `RANDOM_TRACE_ID` flag (`0x02`) set on freshly generated root spans
- Multiple `traceparent` headers → `TraceParentError::MultipleValues`
- Response propagation via `Server-Timing: trace;desc=<traceparent>` (Level 3)
- Optional `serde` feature: `Serialize`/`Deserialize` for all public types

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.
