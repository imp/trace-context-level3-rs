//! W3C Trace Context Level 3 — axum extractor.
//!
//! Provides [`TraceContext`], a thin newtype over
//! [`trace_context_level3_http::TraceContext`] that implements axum's
//! [`FromRequestParts`].
//!
//! If [`trace-context-level3-tower`] middleware is active it stores the
//! context in request extensions; the extractor returns that copy directly
//! (the already-advanced child span). Without the middleware it falls back to
//! parsing `traceparent`/`tracestate` from the raw headers.
//!
//! # Example
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
//! let req = Request::builder()
//!     .uri("/")
//!     .header("traceparent", "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
//!     .body(Body::empty())
//!     .unwrap();
//!
//! let resp = app.oneshot(req).await.unwrap();
//! assert_eq!(resp.status(), StatusCode::OK);
//! # }
//! ```

use std::ops::Deref;

use axum_core::extract::FromRequestParts;
use axum_core::response::IntoResponse;
use axum_core::response::Response;
use http::StatusCode;
use http::request::Parts;
use trace_context_level3::TraceParentError;
use trace_context_level3_http::TraceContext as HttpTraceContext;

/// Axum extractor for W3C Trace Context.
///
/// Wraps [`trace_context_level3_http::TraceContext`]. Use [`Deref`] or `.0`
/// to access the inner value, or `into()` to convert back.
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

/// Rejection returned when [`TraceContext`] extraction fails.
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

impl<S> FromRequestParts<S> for TraceContext
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

    async fn status(router: Router, req: Request<Body>) -> StatusCode {
        router.oneshot(req).await.unwrap().status()
    }

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
    async fn rejects_missing_traceparent() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert_eq!(status(app(), req).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_malformed_traceparent() {
        let req = Request::builder()
            .uri("/")
            .header(TRACEPARENT, "not-a-traceparent")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app(), req).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn reads_from_extension_when_present() {
        use trace_context_level3::TraceState;
        let ctx = HttpTraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        // Simulate what the tower middleware does: store context in extensions.
        let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ctx);

        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), VALID_TP.as_bytes());
    }
}
