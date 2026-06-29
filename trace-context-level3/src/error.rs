/// Errors from parsing or constructing a [`TraceParent`][crate::TraceParent].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TraceParentError {
    /// Header is shorter than 55 characters; the trace must be restarted.
    #[error("traceparent too short: {0} chars (need at least 55)")]
    TooShort(usize),

    /// Version byte `0xff` is reserved and always invalid.
    #[error("traceparent version 0xff is reserved and invalid")]
    InvalidVersion,

    /// A field contained a character outside lowercase hex (`0-9`, `a-f`).
    #[error("invalid character in traceparent (only lowercase hex 0-9 a-f is accepted)")]
    InvalidHex,

    /// A required `-` separator was absent or replaced by another character.
    #[error("missing '-' separator in traceparent")]
    MissingSeparator,

    /// `trace-id` must not be all zeros.
    #[error("trace-id must not be all zeros")]
    ZeroTraceId,

    /// `parent-id` must not be all zeros.
    #[error("parent-id must not be all zeros")]
    ZeroParentId,

    /// For version `00` the header must be exactly 55 characters.
    #[error("traceparent version 00 must be exactly 55 characters, got {0}")]
    TrailingData(usize),
}

/// Errors from parsing or mutating a [`TraceState`][crate::TraceState].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TraceStateError {
    /// The list already contains 32 entries; no more can be added.
    #[error("tracestate cannot hold more than 32 entries")]
    TooManyEntries,

    /// A key did not conform to the Level 2/3 key grammar.
    #[error("invalid tracestate key: {0:?}")]
    InvalidKey(String),

    /// A value contained forbidden characters or violated length constraints.
    #[error("invalid tracestate value: {0:?}")]
    InvalidValue(String),

    /// The same key appeared more than once in the `tracestate` header.
    #[error("duplicate tracestate key: {0:?}")]
    DuplicateKey(String),
}
