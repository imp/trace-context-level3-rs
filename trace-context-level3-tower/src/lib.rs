//! W3C Trace Context Level 3 — tower middleware storing context in request extensions.
//!
//! [`TraceContextLayer`] extracts the incoming `traceparent`/`tracestate` headers,
//! advances the span (or starts a new root), and stores the resulting
//! [`TraceContext`] in [`http::Request`] extensions for downstream handlers.
//!
//! With the `task-local` feature enabled, calling
//! [`TraceContextLayer::enable_task_local`] additionally stores the context in the
//! [`TRACE_CONTEXT`] task-local for the duration of each inner future — no second
//! header parse.

use std::future::Future;
use std::pin::Pin;
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

#[cfg(feature = "task-local")]
tokio::task_local! {
    /// Task-local [`TraceContext`] set by [`TraceContextLayer`] when enabled via
    /// [`TraceContextLayer::enable_task_local`].
    pub static TRACE_CONTEXT: TraceContext;
}

/// Future returned by [`TraceContextService`].
///
/// Always wraps the inner future. When the `task-local` feature is enabled and
/// the layer was built with [`TraceContextLayer::enable_task_local`], the inner
/// future runs inside a [`TRACE_CONTEXT`] scope.
#[pin_project::pin_project(project = TraceContextFutureProj)]
pub enum TraceContextFuture<F> {
    Plain(#[pin] F),
    #[cfg(feature = "task-local")]
    Scoped(#[pin] tokio::task::futures::TaskLocalFuture<TraceContext, F>),
}

impl<F> std::fmt::Debug for TraceContextFuture<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceContextFuture").finish_non_exhaustive()
    }
}

impl<F: Future> Future for TraceContextFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            TraceContextFutureProj::Plain(fut) => fut.poll(cx),
            #[cfg(feature = "task-local")]
            TraceContextFutureProj::Scoped(fut) => fut.poll(cx),
        }
    }
}

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
    #[cfg(feature = "task-local")]
    task_local: bool,
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
        Self {
            generator,
            #[cfg(feature = "task-local")]
            task_local: false,
        }
    }
}

#[cfg(feature = "task-local")]
impl<G> TraceContextLayer<G> {
    /// Also stores the [`TraceContext`] in the [`TRACE_CONTEXT`] task-local for
    /// the duration of each inner future.
    pub fn enable_task_local(mut self) -> Self {
        self.task_local = true;
        self
    }
}

impl<S, G: IdGenerator + Clone> Layer<S> for TraceContextLayer<G> {
    type Service = TraceContextService<S, G>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceContextService {
            inner,
            generator: self.generator.clone(),
            #[cfg(feature = "task-local")]
            task_local: self.task_local,
        }
    }
}

/// Tower [`Service`] produced by [`TraceContextLayer`].
#[derive(Clone, Debug)]
pub struct TraceContextService<S, G = RandIdGenerator> {
    inner: S,
    generator: G,
    #[cfg(feature = "task-local")]
    task_local: bool,
}

impl<S, G, B> Service<Request<B>> for TraceContextService<S, G>
where
    S: Service<Request<B>>,
    G: IdGenerator,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = TraceContextFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        let ctx = build_trace_context(&self.generator, req.headers());
        #[cfg(feature = "task-local")]
        if self.task_local {
            req.extensions_mut().insert(ctx.clone());
            return TraceContextFuture::Scoped(TRACE_CONTEXT.scope(ctx, self.inner.call(req)));
        }
        req.extensions_mut().insert(ctx);
        TraceContextFuture::Plain(self.inner.call(req))
    }
}

/// Tower [`Layer`] that injects the current trace context into outgoing
/// request headers, for use on HTTP client stacks.
///
/// On each request the service:
/// 1. Looks for a [`TraceContext`] in request extensions (set explicitly by the caller).
/// 2. Falls back to the [`TRACE_CONTEXT`] task-local (if the `task-local` feature is enabled).
/// 3. If a context is found: advances the span (new `parent-id`) and injects
///    `traceparent` / `tracestate` headers into the outgoing request.
/// 4. If no context is found: forwards the request unchanged.
///
/// # Example
///
/// ```rust,no_run
/// use trace_context_level3_tower::TraceContextClientLayer;
///
/// let layer = TraceContextClientLayer::new();
/// ```
#[derive(Clone, Debug, Default)]
pub struct TraceContextClientLayer<G = RandIdGenerator> {
    generator: G,
}

impl TraceContextClientLayer<RandIdGenerator> {
    /// Creates a new [`TraceContextClientLayer`] using the default [`RandIdGenerator`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<G> TraceContextClientLayer<G> {
    /// Creates a [`TraceContextClientLayer`] with a custom [`IdGenerator`].
    pub fn with_generator(generator: G) -> Self {
        Self { generator }
    }
}

impl<S, G: IdGenerator + Clone> Layer<S> for TraceContextClientLayer<G> {
    type Service = TraceContextClientService<S, G>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceContextClientService {
            inner,
            generator: self.generator.clone(),
        }
    }
}

/// Tower [`Service`] produced by [`TraceContextClientLayer`].
#[derive(Clone, Debug)]
pub struct TraceContextClientService<S, G = RandIdGenerator> {
    inner: S,
    generator: G,
}

impl<S, G, B> Service<Request<B>> for TraceContextClientService<S, G>
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
        if let Some(ctx) = current_trace_context(&req) {
            let outgoing = TraceContext {
                traceparent: ctx.traceparent.child(self.generator.new_parent_id()),
                tracestate: ctx.tracestate,
            };
            outgoing.inject(req.headers_mut());
        }
        self.inner.call(req)
    }
}

/// Returns the active [`TraceContext`] for the current request: checks request
/// extensions first, then falls back to the task-local (if the feature is on).
fn current_trace_context<B>(req: &Request<B>) -> Option<TraceContext> {
    if let Some(ctx) = req.extensions().get::<TraceContext>() {
        return Some(ctx.clone());
    }
    #[cfg(feature = "task-local")]
    if let Ok(ctx) = TRACE_CONTEXT.try_with(|c| c.clone()) {
        return Some(ctx);
    }
    None
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

    #[cfg(feature = "task-local")]
    #[tokio::test]
    async fn task_local_matches_extension_when_enabled() {
        let layer = TraceContextLayer::new().enable_task_local();
        let svc = layer.layer(tower::service_fn(|req: Request<()>| async move {
            let ext = req.extensions().get::<TraceContext>().cloned().unwrap();
            let tl = TRACE_CONTEXT.with(|ctx| ctx.clone());
            assert_eq!(ext, tl);
            Ok::<_, Infallible>(())
        }));
        svc.oneshot(Request::new(())).await.unwrap();
    }

    #[cfg(feature = "task-local")]
    #[tokio::test]
    async fn task_local_not_set_by_default() {
        let layer = TraceContextLayer::new();
        let svc = layer.layer(tower::service_fn(|_req: Request<()>| async move {
            assert!(
                TRACE_CONTEXT.try_with(|_| ()).is_err(),
                "task-local should not be set without enable_task_local()"
            );
            Ok::<_, Infallible>(())
        }));
        svc.oneshot(Request::new(())).await.unwrap();
    }

    // --- TraceContextClientLayer ---

    #[tokio::test]
    async fn client_injects_headers_from_extension() {
        let original: TraceParent = VALID_TP.parse().unwrap();
        let layer = TraceContextClientLayer::new();
        let svc = layer.layer(tower::service_fn(|req: Request<()>| async move {
            Ok::<_, Infallible>(req.headers().clone())
        }));
        let mut req = Request::new(());
        req.extensions_mut().insert(TraceContext {
            traceparent: original,
            tracestate: TraceState::default(),
        });
        let headers = svc.oneshot(req).await.unwrap();
        let injected: TraceParent = headers
            .get("traceparent")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        // Same trace-id, new parent-id.
        assert_eq!(injected.trace_id, original.trace_id);
        assert_ne!(injected.parent_id, original.parent_id);
    }

    #[tokio::test]
    async fn client_no_injection_without_context() {
        let layer = TraceContextClientLayer::new();
        let svc = layer.layer(tower::service_fn(|req: Request<()>| async move {
            Ok::<_, Infallible>(req.headers().clone())
        }));
        let headers = svc.oneshot(Request::new(())).await.unwrap();
        assert!(headers.get("traceparent").is_none());
    }

    #[cfg(feature = "task-local")]
    #[tokio::test]
    async fn client_falls_back_to_task_local() {
        use trace_context_level3::TraceFlags;
        use trace_context_level3::TraceId;

        let tp = TraceParent::restart(
            TraceId::from_bytes([0xAB; 16]).unwrap(),
            trace_context_level3::ParentId::from_bytes([0xCD; 8]).unwrap(),
            TraceFlags::default(),
        );
        let ctx = TraceContext {
            traceparent: tp,
            tracestate: TraceState::default(),
        };
        let layer = TraceContextClientLayer::new();
        let svc = layer.layer(tower::service_fn(|req: Request<()>| async move {
            Ok::<_, Infallible>(req.headers().clone())
        }));
        let headers = TRACE_CONTEXT
            .scope(
                ctx,
                async move { svc.oneshot(Request::new(())).await.unwrap() },
            )
            .await;
        let injected: TraceParent = headers
            .get("traceparent")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(injected.trace_id, tp.trace_id);
        assert_ne!(injected.parent_id, tp.parent_id);
    }
}
