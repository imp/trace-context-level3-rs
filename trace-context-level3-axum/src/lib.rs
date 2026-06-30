//! W3C Trace Context Level 3 — axum extractors.
//!
//! Two extractors are provided:
//!
//! - [`TraceContext`] — spec-compliant default. When `traceparent` is absent or
//!   invalid the W3C spec says to restart the trace; this extractor does exactly
//!   that, generating a fresh root span. Handlers always receive a valid context.
//!   Rejection type is [`Infallible`], so `from_request_parts` never fails.
//!
//! - [`StrictTraceContext`] — policy override. Rejects with `400 Bad Request`
//!   when `traceparent` is absent or invalid. Use this when your service must
//!   refuse requests that don't carry an upstream trace.
//!
//! Both extractors check request extensions first: when
//! [`trace-context-level3-tower`] middleware is in the stack it stores the
//! already-advanced child span there, and neither extractor re-parses the raw
//! headers.
//!
//! Use [`Option<StrictTraceContext>`] when a handler wants to distinguish "context
//! was propagated" from "absent or invalid"; axum's blanket impl converts any
//! rejection to `None`.
//!
//! # Example — spec-compliant (always present)
//!
//! ```rust
//! use axum::Router;
//! use axum::body::Body;
//! use axum::routing::get;
//! use http::{Request, StatusCode};
//! use tower::ServiceExt as _;
//! use trace_context_level3_axum::TraceContext;
//!
//! async fn handler(ctx: TraceContext) -> String {
//!     ctx.traceparent.to_string()
//! }
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let app: Router = Router::new().route("/", get(handler));
//!
//! // No traceparent — handler still gets a fresh root span, not a 400.
//! let req = Request::builder().uri("/").body(Body::empty()).unwrap();
//! let resp = app.clone().oneshot(req).await.unwrap();
//! assert_eq!(resp.status(), StatusCode::OK);
//!
//! // With traceparent — context is extracted and forwarded.
//! let req = Request::builder()
//!     .uri("/")
//!     .header("traceparent", "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
//!     .body(Body::empty())
//!     .unwrap();
//! let resp = app.oneshot(req).await.unwrap();
//! assert_eq!(resp.status(), StatusCode::OK);
//! # }
//! ```

use std::convert::Infallible;
use std::ops::Deref;

use axum_core::extract::FromRequestParts;
use axum_core::response::IntoResponse;
use axum_core::response::Response;
use http::StatusCode;
use http::request::Parts;
use trace_context_level3::TraceFlags;
use trace_context_level3::TraceParent;
use trace_context_level3::TraceParentError;
use trace_context_level3::TraceState;
use trace_context_level3_http::TraceContext as HttpTraceContext;

/// Spec-compliant axum extractor for W3C Trace Context.
///
/// When `traceparent` is absent or invalid a fresh root span is generated
/// (per spec: receivers SHOULD start a new trace when no `traceparent` is
/// present, and MUST restart the trace on an invalid one).
///
/// This extractor never rejects; use [`Option<StrictTraceContext>`] when you
/// need to distinguish "context was propagated" from "was not".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext(pub HttpTraceContext);

impl Deref for TraceContext {
    type Target = HttpTraceContext;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<HttpTraceContext> for TraceContext {
    fn from(ctx: HttpTraceContext) -> Self {
        Self(ctx)
    }
}

impl From<TraceContext> for HttpTraceContext {
    fn from(ctx: TraceContext) -> Self {
        ctx.0
    }
}

impl<S> FromRequestParts<S> for TraceContext
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(ctx) = parts.extensions.get::<HttpTraceContext>() {
            return Ok(Self(ctx.clone()));
        }
        let ctx = match HttpTraceContext::extract(&parts.headers) {
            Ok(Some(ctx)) => ctx,
            // Absent or invalid: start a new trace per spec.
            Ok(None) | Err(_) => HttpTraceContext {
                traceparent: TraceParent::new_root(TraceFlags::SAMPLED),
                tracestate: TraceState::default(),
            },
        };
        Ok(Self(ctx))
    }
}

/// Strict axum extractor for W3C Trace Context.
///
/// Rejects with `400 Bad Request` when `traceparent` is absent or invalid.
/// This is a policy override beyond what the spec requires; use [`TraceContext`]
/// for the spec-compliant default.
///
/// Use [`Option<StrictTraceContext>`] to distinguish "context was propagated"
/// from "absent or invalid" without rejecting the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrictTraceContext(pub HttpTraceContext);

impl Deref for StrictTraceContext {
    type Target = HttpTraceContext;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<HttpTraceContext> for StrictTraceContext {
    fn from(ctx: HttpTraceContext) -> Self {
        Self(ctx)
    }
}

impl From<StrictTraceContext> for HttpTraceContext {
    fn from(ctx: StrictTraceContext) -> Self {
        ctx.0
    }
}

/// Rejection returned when [`StrictTraceContext`] extraction fails.
#[derive(Debug)]
pub enum TraceContextRejection {
    /// No `traceparent` header was present.
    Missing,
    /// A `traceparent` header was present but could not be parsed.
    Invalid(TraceParentError),
}

impl IntoResponse for TraceContextRejection {
    fn into_response(self) -> Response {
        let body = match &self {
            Self::Missing => "missing traceparent header".to_owned(),
            Self::Invalid(e) => format!("invalid traceparent: {e}"),
        };
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}

impl<S> FromRequestParts<S> for StrictTraceContext
where
    S: Send + Sync,
{
    type Rejection = TraceContextRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(ctx) = parts.extensions.get::<HttpTraceContext>() {
            return Ok(Self(ctx.clone()));
        }
        match HttpTraceContext::extract(&parts.headers) {
            Ok(Some(ctx)) => Ok(Self(ctx)),
            Ok(None) => Err(TraceContextRejection::Missing),
            Err(e) => Err(TraceContextRejection::Invalid(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use http::Request;
    use tower::ServiceExt as _;
    use trace_context_level3_http::TRACEPARENT;

    use super::*;

    const VALID_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    fn app() -> Router {
        Router::new().route(
            "/",
            get(|ctx: TraceContext| async move { ctx.traceparent.to_string() }),
        )
    }

    fn strict_app() -> Router {
        Router::new().route(
            "/",
            get(|ctx: StrictTraceContext| async move { ctx.traceparent.to_string() }),
        )
    }

    async fn status(router: Router, req: Request<Body>) -> StatusCode {
        router.oneshot(req).await.unwrap().status()
    }

    // ── TraceContext (spec-compliant) ────────────────────────────────────────

    #[tokio::test]
    async fn extracts_valid_traceparent() {
        let req = Request::builder()
            .uri("/")
            .header(TRACEPARENT, VALID_TP)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app(), req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn generates_fresh_span_when_missing() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert_eq!(status(app(), req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn generates_fresh_span_when_invalid() {
        let req = Request::builder()
            .uri("/")
            .header(TRACEPARENT, "not-a-traceparent")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app(), req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn fresh_span_has_random_trace_id_flag() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let tp: TraceParent = std::str::from_utf8(&body).unwrap().parse().unwrap();
        assert!(
            tp.is_random_trace_id(),
            "fresh root span must set RANDOM_TRACE_ID"
        );
    }

    #[tokio::test]
    async fn reads_from_extension_when_present() {
        let ctx = HttpTraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ctx);

        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), VALID_TP.as_bytes());
    }

    // ── StrictTraceContext ───────────────────────────────────────────────────

    #[tokio::test]
    async fn strict_extracts_valid_traceparent() {
        let req = Request::builder()
            .uri("/")
            .header(TRACEPARENT, VALID_TP)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(strict_app(), req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn strict_rejects_missing_traceparent() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert_eq!(status(strict_app(), req).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn strict_rejects_malformed_traceparent() {
        let req = Request::builder()
            .uri("/")
            .header(TRACEPARENT, "not-a-traceparent")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(strict_app(), req).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn strict_reads_from_extension_when_present() {
        let ctx = HttpTraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ctx);

        let resp = strict_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), VALID_TP.as_bytes());
    }
}
