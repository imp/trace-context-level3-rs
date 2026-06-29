//! W3C Trace Context Level 3 — tower middleware storing context in request extensions.
//!
//! [`TraceContextLayer`] extracts the incoming `traceparent`/`tracestate` headers,
//! advances the span (or starts a new root), and stores the resulting
//! [`TraceContext`] in [`http::Request`] extensions for downstream handlers.

use std::task::Context;
use std::task::Poll;

use http::HeaderMap;
use http::Request;
use tower::Layer;
use tower::Service;
use trace_context_level3::IdGenerator;
use trace_context_level3::ParentId;
use trace_context_level3::TraceFlags;
use trace_context_level3::TraceId;
use trace_context_level3::TraceParent;
use trace_context_level3::TraceState;
use trace_context_level3_http::TraceContext;

/// An [`IdGenerator`] backed by `rand`, producing cryptographically random IDs.
///
/// This is the default generator used by [`TraceContextLayer::new`].
#[derive(Clone, Copy, Debug, Default)]
pub struct RandIdGenerator;

impl IdGenerator for RandIdGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::random()
    }

    fn new_parent_id(&self) -> ParentId {
        ParentId::random()
    }
}

/// Tower [`Layer`] that extracts trace context from incoming request headers and
/// stores it as a [`TraceContext`] request extension.
///
/// On each request the middleware:
/// 1. Extracts `traceparent` / `tracestate` from headers.
/// 2. If present and valid: advances the span by generating a new `parent-id`.
/// 3. If absent or invalid: starts a fresh root span with new random IDs.
/// 4. Stores the resulting [`TraceContext`] in `req.extensions()`.
///
/// # Example
///
/// ```rust,no_run
/// use trace_context_level3_tower::TraceContextLayer;
///
/// let layer = TraceContextLayer::new();
/// ```
#[derive(Clone, Debug, Default)]
pub struct TraceContextLayer<G = RandIdGenerator> {
    generator: G,
}

impl TraceContextLayer<RandIdGenerator> {
    /// Creates a new [`TraceContextLayer`] using the default [`RandIdGenerator`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<G> TraceContextLayer<G> {
    /// Creates a [`TraceContextLayer`] with a custom [`IdGenerator`].
    pub fn with_generator(generator: G) -> Self {
        Self { generator }
    }
}

impl<S, G: IdGenerator + Clone> Layer<S> for TraceContextLayer<G> {
    type Service = TraceContextService<S, G>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceContextService {
            inner,
            generator: self.generator.clone(),
        }
    }
}

/// Tower [`Service`] produced by [`TraceContextLayer`].
#[derive(Clone, Debug)]
pub struct TraceContextService<S, G = RandIdGenerator> {
    inner: S,
    generator: G,
}

impl<S, G, B> Service<Request<B>> for TraceContextService<S, G>
where
    S: Service<Request<B>>,
    G: IdGenerator,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let ctx = build_trace_context(&self.generator, req.headers());
        req.extensions_mut().insert(ctx);
        self.inner.call(req)
    }
}

fn build_trace_context<G: IdGenerator>(generator: &G, headers: &HeaderMap) -> TraceContext {
    match TraceContext::extract(headers) {
        Some(Ok(ctx)) => TraceContext {
            traceparent: ctx.traceparent.child(generator.new_parent_id()),
            tracestate: ctx.tracestate,
        },
        _ => {
            let flags = if G::RANDOM {
                TraceFlags::RANDOM_TRACE_ID
            } else {
                TraceFlags::default()
            };
            TraceContext {
                traceparent: TraceParent::restart(
                    generator.new_trace_id(),
                    generator.new_parent_id(),
                    flags,
                ),
                tracestate: TraceState::default(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use http::HeaderName;
    use http::HeaderValue;
    use tower::ServiceExt;

    use super::*;

    const VALID_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    async fn run<G: IdGenerator + Clone>(
        layer: TraceContextLayer<G>,
        headers: &[(&str, &str)],
    ) -> TraceContext {
        let svc = layer.layer(tower::service_fn(|req: Request<()>| async move {
            Ok::<_, Infallible>(req.extensions().get::<TraceContext>().cloned().unwrap())
        }));
        let mut req = Request::new(());
        for (name, value) in headers {
            req.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        svc.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn advances_span_on_valid_traceparent() {
        let ctx = run(TraceContextLayer::new(), &[("traceparent", VALID_TP)]).await;
        let original: TraceParent = VALID_TP.parse().unwrap();
        assert_eq!(ctx.traceparent.trace_id, original.trace_id);
        assert_ne!(ctx.traceparent.parent_id, original.parent_id);
        assert_eq!(ctx.traceparent.trace_flags, original.trace_flags);
    }

    #[tokio::test]
    async fn creates_root_span_when_no_traceparent() {
        let ctx = run(TraceContextLayer::new(), &[]).await;
        assert!(ctx.traceparent.is_random_trace_id());
        assert!(ctx.tracestate.is_empty());
    }

    #[tokio::test]
    async fn creates_root_span_on_invalid_traceparent() {
        let ctx = run(TraceContextLayer::new(), &[("traceparent", "garbage")]).await;
        assert!(ctx.traceparent.is_random_trace_id());
    }

    #[tokio::test]
    async fn preserves_tracestate() {
        let ctx = run(
            TraceContextLayer::new(),
            &[("traceparent", VALID_TP), ("tracestate", "vendor=val")],
        )
        .await;
        assert_eq!(ctx.tracestate.get("vendor"), Some("val"));
    }

    #[tokio::test]
    async fn custom_generator_skips_random_flag() {
        #[derive(Clone)]
        struct SeqGen {
            trace_id: TraceId,
            parent_id: ParentId,
        }
        impl IdGenerator for SeqGen {
            const RANDOM: bool = false;
            fn new_trace_id(&self) -> TraceId {
                self.trace_id
            }
            fn new_parent_id(&self) -> ParentId {
                self.parent_id
            }
        }

        let generator = SeqGen {
            trace_id: TraceId::from_bytes([0x01; 16]).unwrap(),
            parent_id: ParentId::from_bytes([0x02; 8]).unwrap(),
        };
        let layer = TraceContextLayer::with_generator(generator);
        let ctx = run(layer, &[]).await;
        assert!(!ctx.traceparent.is_random_trace_id());
    }
}
