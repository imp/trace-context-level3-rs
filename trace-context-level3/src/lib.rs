//! W3C Trace Context Level 3 implementation.
//!
//! Parses and serializes the [`traceparent`][TraceParent] and
//! [`tracestate`][TraceState] HTTP headers as defined in the
//! [W3C Trace Context specification](https://w3c.github.io/trace-context/).

use std::fmt;
use std::ops;
use std::str;

pub use error::TraceParentError;
pub use error::TraceStateError;
pub use traceparent::ParentId;
pub use traceparent::TraceFlags;
pub use traceparent::TraceId;
pub use traceparent::TraceParent;
pub use tracestate::TraceState;

mod error;
mod traceparent;
mod tracestate;

/// Generates new [`TraceId`] and [`ParentId`] values for span creation.
///
/// Implement this trait to plug in a custom entropy source. The blanket
/// default uses `rand`; set [`IdGenerator::RANDOM`] to `false` for
/// deterministic or sequential generators so the middleware can omit the
/// [`TraceFlags::RANDOM_TRACE_ID`] flag on new root spans.
pub trait IdGenerator {
    /// `true` when generated IDs contain random bytes.
    ///
    /// The middleware uses this to decide whether to set
    /// [`TraceFlags::RANDOM_TRACE_ID`] when starting a root span.
    const RANDOM: bool = true;

    fn new_trace_id(&self) -> TraceId;
    fn new_parent_id(&self) -> ParentId;
}
