//! W3C Trace Context Level 3 — HTTP header extraction and injection.
//!
//! Provides [`TraceContext`] for reading and writing the [`TRACEPARENT`] and
//! [`TRACESTATE`] headers, plus the header name constants themselves.

use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use trace_context_level3::TraceParent;
use trace_context_level3::TraceParentError;
use trace_context_level3::TraceState;

/// The `traceparent` header name.
pub const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");

/// The `tracestate` header name.
pub const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");

/// The trace context extracted from HTTP headers: `traceparent` and
/// `tracestate` treated as a single propagation unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    pub traceparent: TraceParent,
    pub tracestate: TraceState,
}

impl TraceContext {
    /// Extracts the trace context from HTTP headers.
    ///
    /// Returns `None` if no `traceparent` header is present.
    /// Returns `Some(Err(_))` if `traceparent` is present but malformed.
    /// A missing or malformed `tracestate` is treated leniently as empty,
    /// per the spec's guidance for intermediaries.
    pub fn extract(headers: &HeaderMap) -> Option<Result<Self, TraceParentError>> {
        let tp_str = headers.get(&TRACEPARENT)?.to_str().ok()?;
        let traceparent = match tp_str.parse::<TraceParent>() {
            Ok(tp) => tp,
            Err(e) => return Some(Err(e)),
        };
        let tracestate = collect_tracestate(headers);
        Some(Ok(Self {
            traceparent,
            tracestate,
        }))
    }

    /// Injects the trace context into HTTP headers, overwriting any existing
    /// `traceparent` and `tracestate` values.
    ///
    /// An empty `tracestate` removes the header rather than writing a blank value.
    pub fn inject(&self, headers: &mut HeaderMap) {
        let tp_val = HeaderValue::from_str(&self.traceparent.to_string())
            .expect("traceparent is always valid ASCII");
        headers.insert(TRACEPARENT.clone(), tp_val);

        if self.tracestate.is_empty() {
            headers.remove(&TRACESTATE);
        } else {
            let ts_val = HeaderValue::from_str(&self.tracestate.to_string())
                .expect("tracestate is always valid ASCII");
            headers.insert(TRACESTATE.clone(), ts_val);
        }
    }
}

/// Concatenates all `tracestate` header values with a comma separator,
/// as required when multiple `tracestate` headers are present.
/// Falls back to an empty `TraceState` if the combined value cannot be parsed.
fn collect_tracestate(headers: &HeaderMap) -> TraceState {
    let combined = headers
        .get_all(&TRACESTATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    combined.parse().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    const VALID_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn extract_returns_none_with_no_traceparent() {
        let headers = HeaderMap::new();
        assert!(TraceContext::extract(&headers).is_none());
    }

    #[test]
    fn extract_parses_traceparent() {
        let headers = header_map(&[("traceparent", VALID_TP)]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert!(ctx.traceparent.is_sampled());
        assert!(ctx.tracestate.is_empty());
    }

    #[test]
    fn extract_returns_err_on_invalid_traceparent() {
        let headers = header_map(&[("traceparent", "not-a-traceparent")]);
        assert!(matches!(TraceContext::extract(&headers), Some(Err(_))));
    }

    #[test]
    fn extract_parses_tracestate() {
        let headers = header_map(&[("traceparent", VALID_TP), ("tracestate", "vendor=value")]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert_eq!(ctx.tracestate.get("vendor"), Some("value"));
    }

    #[test]
    fn extract_ignores_invalid_tracestate() {
        let headers = header_map(&[("traceparent", VALID_TP), ("tracestate", "!!!invalid!!!")]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert!(ctx.tracestate.is_empty());
    }

    #[test]
    fn inject_writes_traceparent() {
        let ctx = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        let mut headers = HeaderMap::new();
        ctx.inject(&mut headers);
        assert_eq!(headers.get("traceparent").unwrap(), VALID_TP);
        assert!(headers.get("tracestate").is_none());
    }

    #[test]
    fn inject_writes_tracestate_when_nonempty() {
        let mut ts = TraceState::default();
        ts.insert("vendor", "value").unwrap();
        let ctx = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: ts,
        };
        let mut headers = HeaderMap::new();
        ctx.inject(&mut headers);
        assert_eq!(
            headers.get("tracestate").unwrap().to_str().unwrap(),
            "vendor=value"
        );
    }

    #[test]
    fn inject_removes_tracestate_when_empty() {
        let mut headers = header_map(&[("tracestate", "vendor=old")]);
        let ctx = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        ctx.inject(&mut headers);
        assert!(headers.get("tracestate").is_none());
    }

    #[test]
    fn roundtrip_inject_then_extract() {
        let original = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: {
                let mut ts = TraceState::default();
                ts.insert("vendor", "data").unwrap();
                ts
            },
        };
        let mut headers = HeaderMap::new();
        original.inject(&mut headers);
        let recovered = TraceContext::extract(&headers).unwrap().unwrap();
        assert_eq!(recovered, original);
    }
}
