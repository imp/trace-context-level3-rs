//! W3C Trace Context Level 3 implementation.
//!
//! Parses and serializes the [`traceparent`][TraceParent] and
//! [`tracestate`][TraceState] HTTP headers as defined in the
//! [W3C Trace Context specification](https://w3c.github.io/trace-context/).

use std::fmt;
use std::ops;
use std::str;

pub use error::{TraceParentError, TraceStateError};
pub use traceparent::{ParentId, TraceFlags, TraceId, TraceParent};
pub use tracestate::TraceState;

mod error;
mod traceparent;
mod tracestate;
