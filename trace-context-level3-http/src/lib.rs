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
    /// Returns `Ok(None)` if no `traceparent` header is present.
    /// Returns `Err(_)` if `traceparent` is present but malformed.
    /// A missing or malformed `tracestate` is treated leniently as empty,
    /// per the spec's guidance for intermediaries.
    pub fn extract(headers: &HeaderMap) -> Result<Option<Self>, TraceParentError> {
        let mut tp_values = headers.get_all(&TRACEPARENT).iter();
        let Some(first) = tp_values.next() else {
            return Ok(None);
        };
        if tp_values.next().is_some() {
            return Err(TraceParentError::MultipleValues);
        }
        let Some(tp_str) = first.to_str().ok() else {
            return Ok(None);
        };
        let traceparent = tp_str.parse::<TraceParent>()?;
        let tracestate = collect_tracestate(headers);
        Ok(Some(Self {
            traceparent,
            tracestate,
        }))
    }

    /// Injects the trace context into HTTP headers, overwriting any existing
    /// `traceparent` and `tracestate` values.
    ///
    /// An empty `tracestate` removes the header rather than writing a blank value.
    pub fn inject(&self, headers: &mut HeaderMap) {
        let traceparent = HeaderValue::from_str(&self.traceparent.to_string())
            .expect("traceparent is always valid ASCII");
        headers.insert(TRACEPARENT.clone(), traceparent);

        let mut ts = self.tracestate.clone();
        ts.truncate(512);
        if ts.is_empty() {
            headers.remove(&TRACESTATE);
        } else {
            let tracestate =
                HeaderValue::from_str(&ts.to_string()).expect("tracestate is always valid ASCII");
            headers.insert(TRACESTATE.clone(), tracestate);
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
    TraceState::parse_lenient(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_map(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    const VALID_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn extract_returns_none_with_no_traceparent() {
        let headers = HeaderMap::new();
        assert_eq!(TraceContext::extract(&headers).unwrap(), None);
    }

    #[test]
    fn extract_parses_traceparent() {
        let headers = header_map(&[(TRACEPARENT, VALID_TP)]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert!(ctx.traceparent.is_sampled());
        assert!(ctx.tracestate.is_empty());
    }

    #[test]
    fn extract_returns_err_on_invalid_traceparent() {
        let headers = header_map(&[(TRACEPARENT, "not-a-traceparent")]);
        assert!(TraceContext::extract(&headers).is_err());
    }

    #[test]
    fn extract_parses_tracestate() {
        let headers = header_map(&[(TRACEPARENT, VALID_TP), (TRACESTATE, "vendor=value")]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert_eq!(ctx.tracestate.get("vendor"), Some("value"));
    }

    #[test]
    fn extract_preserves_valid_entries_alongside_invalid() {
        // A bad entry should be silently dropped; the good one must survive.
        let headers = header_map(&[
            (TRACEPARENT, VALID_TP),
            (TRACESTATE, "vendor=value,!!!bad!!!"),
        ]);
        let ctx = TraceContext::extract(&headers).unwrap().unwrap();
        assert_eq!(ctx.tracestate.get("vendor"), Some("value"));
    }

    #[test]
    fn extract_ignores_invalid_tracestate() {
        let headers = header_map(&[(TRACEPARENT, VALID_TP), (TRACESTATE, "!!!invalid!!!")]);
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
        assert_eq!(headers.get(&TRACEPARENT).unwrap(), VALID_TP);
        assert!(headers.get(&TRACESTATE).is_none());
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
            headers.get(&TRACESTATE).unwrap().to_str().unwrap(),
            "vendor=value"
        );
    }

    #[test]
    fn inject_removes_tracestate_when_empty() {
        let mut headers = header_map(&[(TRACESTATE, "vendor=old")]);
        let ctx = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: TraceState::default(),
        };
        ctx.inject(&mut headers);
        assert!(headers.get(&TRACESTATE).is_none());
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

    #[test]
    fn extract_errors_on_multiple_traceparent_headers() {
        use http::HeaderValue;
        use trace_context_level3::TraceParentError;
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT, HeaderValue::from_static(VALID_TP));
        // append adds a second value for the same header name
        headers.append(
            TRACEPARENT,
            HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"),
        );
        let err = TraceContext::extract(&headers).unwrap_err();
        assert_eq!(err, TraceParentError::MultipleValues);
    }

    #[test]
    fn inject_truncates_oversized_tracestate() {
        // Build a tracestate whose serialised form exceeds 512 bytes.
        // Each entry is "v{i}={pad}" = key 2 chars + '=' + value up to 128 = 131 chars + comma.
        // 5 entries × 103 chars each = 515 serialised bytes.
        let mut ts = TraceState::default();
        // Insert 5 entries with ~103-char values so total > 512 (they get prepended).
        for i in 0..5_u8 {
            let key = format!("v{i}");
            let value = "x".repeat(100);
            ts.insert(&key, &value).unwrap();
        }
        // Sanity: raw serialised length exceeds 512.
        assert!(ts.to_string().len() > 512);

        let ctx = TraceContext {
            traceparent: VALID_TP.parse().unwrap(),
            tracestate: ts,
        };
        let mut headers = HeaderMap::new();
        ctx.inject(&mut headers);

        let written = headers.get(&TRACESTATE).unwrap().to_str().unwrap();
        assert!(
            written.len() <= 512,
            "injected tracestate length {} exceeds 512",
            written.len()
        );
    }
}
