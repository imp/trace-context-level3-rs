//! W3C Trace Context Level 3 — HTTP header extraction and injection.
//!
//! Provides [`TraceContext`] for reading and writing the [`TRACEPARENT`] and
//! [`TRACESTATE`] request headers, plus [`inject_server_timing`] /
//! [`extract_server_timing`] for the `Server-Timing: trace;desc=…` response
//! header defined in the Level 3 spec.

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

/// The `server-timing` response header name.
///
/// W3C Trace Context Level 3 carries the response trace context as the `trace`
/// metric of this standard HTTP header:
/// `Server-Timing: trace;desc=<traceparent-value>`.
pub const SERVER_TIMING: HeaderName = HeaderName::from_static("server-timing");

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
        let mut traceparents = headers.get_all(&TRACEPARENT).iter();
        let Some(first) = traceparents.next() else {
            return Ok(None);
        };
        if traceparents.next().is_some() {
            return Err(TraceParentError::MultipleValues);
        }
        let Some(traceparent) = first.to_str().ok() else {
            return Ok(None);
        };
        let traceparent = traceparent.parse()?;
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

/// Injects the trace context into HTTP response headers as a
/// `Server-Timing: trace;desc=<traceparent>` metric.
///
/// Uses `append` so any existing `Server-Timing` headers are preserved.
///
/// # Example
///
/// ```
/// use http::HeaderMap;
/// use trace_context_level3::TraceParent;
/// use trace_context_level3_http::{SERVER_TIMING, inject_server_timing};
///
/// let tp: TraceParent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
///     .parse()
///     .unwrap();
/// let mut headers = HeaderMap::new();
/// inject_server_timing(&tp, &mut headers);
/// let v = headers.get(SERVER_TIMING).unwrap().to_str().unwrap();
/// assert!(v.starts_with("trace;desc=00-4bf92f3577b34da6a3ce929d0e0e4736"));
/// ```
pub fn inject_server_timing(traceparent: &TraceParent, headers: &mut HeaderMap) {
    let value = format!("trace;desc={traceparent}");
    if let Ok(v) = HeaderValue::from_str(&value) {
        headers.append(SERVER_TIMING.clone(), v);
    }
}

/// Extracts a trace context from `Server-Timing` HTTP response headers.
///
/// Searches all `Server-Timing` header values for a metric named `trace` and
/// parses its `desc` parameter as a `traceparent` value. Metric name and
/// parameter name matching are ASCII case-insensitive per the HTTP spec.
/// Returns `None` when no valid `trace` metric is found.
///
/// # Example
///
/// ```
/// use http::{HeaderMap, HeaderValue};
/// use trace_context_level3_http::{SERVER_TIMING, extract_server_timing};
///
/// let mut headers = HeaderMap::new();
/// headers.insert(
///     SERVER_TIMING,
///     HeaderValue::from_static(
///         "trace;desc=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
///     ),
/// );
/// let tp = extract_server_timing(&headers).unwrap();
/// assert_eq!(tp.trace_id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
/// ```
pub fn extract_server_timing(headers: &HeaderMap) -> Option<TraceParent> {
    headers
        .get_all(&SERVER_TIMING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find_map(parse_trace_timing_metric)
}

/// Parses one Server-Timing metric entry and returns the `TraceParent` from
/// the `desc` parameter when the metric is named `trace`.
fn parse_trace_timing_metric(metric: &str) -> Option<TraceParent> {
    let metric = metric.trim();
    let (name, params) = metric.split_once(';')?;
    if !name.trim().eq_ignore_ascii_case("trace") {
        return None;
    }
    params.split(';').find_map(|param| {
        let (key, val) = param.trim().split_once('=')?;
        if !key.trim().eq_ignore_ascii_case("desc") {
            return None;
        }
        val.trim().trim_matches('"').parse::<TraceParent>().ok()
    })
}

/// Concatenates all `tracestate` header values with a comma separator,
/// as required when multiple `tracestate` headers are present.
/// Falls back to an empty `TraceState` if the combined value cannot be parsed.
fn collect_tracestate(headers: &HeaderMap) -> TraceState {
    let values = headers
        .get_all(&TRACESTATE)
        .iter()
        .filter_map(|v| v.to_str().ok());
    TraceState::parse_lenient_many(values)
}

#[cfg(feature = "serde")]
mod serde {
    use std::fmt;

    use serde_core::Deserialize;
    use serde_core::Deserializer;
    use serde_core::Serialize;
    use serde_core::Serializer;
    use serde_core::de;
    use serde_core::de::MapAccess;
    use serde_core::de::Visitor;
    use serde_core::ser::SerializeStruct;

    use super::TraceContext;

    impl Serialize for TraceContext {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut state = s.serialize_struct("TraceContext", 2)?;
            state.serialize_field("traceparent", &self.traceparent)?;
            state.serialize_field("tracestate", &self.tracestate)?;
            state.end()
        }
    }

    impl<'de> Deserialize<'de> for TraceContext {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            enum Field {
                Traceparent,
                Tracestate,
            }

            impl<'de> Deserialize<'de> for Field {
                fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    struct FieldVisitor;
                    impl<'de> Visitor<'de> for FieldVisitor {
                        type Value = Field;
                        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                            f.write_str("`traceparent` or `tracestate`")
                        }
                        fn visit_str<E: de::Error>(self, s: &str) -> Result<Field, E> {
                            match s {
                                "traceparent" => Ok(Field::Traceparent),
                                "tracestate" => Ok(Field::Tracestate),
                                _ => Err(E::unknown_field(s, FIELDS)),
                            }
                        }
                    }
                    d.deserialize_identifier(FieldVisitor)
                }
            }

            struct TraceContextVisitor;
            impl<'de> Visitor<'de> for TraceContextVisitor {
                type Value = TraceContext;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("struct TraceContext")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let mut traceparent = None;
                    let mut tracestate = None;
                    while let Some(key) = map.next_key()? {
                        match key {
                            Field::Traceparent => {
                                if traceparent.is_some() {
                                    return Err(de::Error::duplicate_field("traceparent"));
                                }
                                traceparent = Some(map.next_value()?);
                            }
                            Field::Tracestate => {
                                if tracestate.is_some() {
                                    return Err(de::Error::duplicate_field("tracestate"));
                                }
                                tracestate = Some(map.next_value()?);
                            }
                        }
                    }
                    Ok(TraceContext {
                        traceparent: traceparent
                            .ok_or_else(|| de::Error::missing_field("traceparent"))?,
                        tracestate: tracestate
                            .ok_or_else(|| de::Error::missing_field("tracestate"))?,
                    })
                }
            }

            const FIELDS: &[&str] = &["traceparent", "tracestate"];
            d.deserialize_struct("TraceContext", FIELDS, TraceContextVisitor)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::*;

        const VALID_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

        #[test]
        fn trace_context_roundtrip_json() {
            let ctx = TraceContext {
                traceparent: VALID_TP.parse().unwrap(),
                tracestate: "vendor=value".parse().unwrap(),
            };
            let json = serde_json::to_string(&ctx).unwrap();
            assert!(json.contains(VALID_TP));
            assert!(json.contains("vendor=value"));
            let back: TraceContext = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ctx);
        }

        #[test]
        fn trace_context_empty_tracestate_roundtrip() {
            let ctx = TraceContext {
                traceparent: VALID_TP.parse().unwrap(),
                tracestate: TraceState::default(),
            };
            let json = serde_json::to_string(&ctx).unwrap();
            let back: TraceContext = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ctx);
            assert!(back.tracestate.is_empty());
        }

        #[test]
        fn trace_context_json_shape() {
            let ctx = TraceContext {
                traceparent: VALID_TP.parse().unwrap(),
                tracestate: TraceState::default(),
            };
            let v: serde_json::Value = serde_json::to_value(&ctx).unwrap();
            assert_eq!(v["traceparent"], serde_json::Value::String(VALID_TP.into()));
            assert_eq!(v["tracestate"], serde_json::Value::String(String::new()));
        }
    }
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

    // ── Server-Timing (traceresponse) ────────────────────────────────────────

    #[test]
    fn inject_server_timing_writes_trace_metric() {
        let tp: TraceParent = VALID_TP.parse().unwrap();
        let mut headers = HeaderMap::new();
        inject_server_timing(&tp, &mut headers);
        let v = headers.get(&SERVER_TIMING).unwrap().to_str().unwrap();
        assert_eq!(v, format!("trace;desc={VALID_TP}"));
    }

    #[test]
    fn inject_server_timing_appends_not_replaces() {
        let tp: TraceParent = VALID_TP.parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_static("db;dur=53,app;dur=47.2"),
        );
        inject_server_timing(&tp, &mut headers);
        // Two Server-Timing entries: original + the trace one.
        let values: Vec<_> = headers
            .get_all(&SERVER_TIMING)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(values.len(), 2);
        assert!(values.iter().any(|v| v.starts_with("trace;desc=")));
        assert!(values.iter().any(|v| v.starts_with("db;")));
    }

    #[test]
    fn extract_server_timing_parses_trace_metric() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_str(&format!("trace;desc={VALID_TP}")).unwrap(),
        );
        let tp = extract_server_timing(&headers).unwrap();
        assert_eq!(tp.to_string(), VALID_TP);
    }

    #[test]
    fn extract_server_timing_finds_trace_among_other_metrics() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_str(&format!("db;dur=53,trace;desc={VALID_TP},app;dur=12")).unwrap(),
        );
        let tp = extract_server_timing(&headers).unwrap();
        assert_eq!(tp.to_string(), VALID_TP);
    }

    #[test]
    fn extract_server_timing_case_insensitive_metric_name() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_str(&format!("Trace;desc={VALID_TP}")).unwrap(),
        );
        assert!(extract_server_timing(&headers).is_some());
    }

    #[test]
    fn extract_server_timing_handles_quoted_desc() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_str(&format!("trace;desc=\"{VALID_TP}\"")).unwrap(),
        );
        let tp = extract_server_timing(&headers).unwrap();
        assert_eq!(tp.to_string(), VALID_TP);
    }

    #[test]
    fn extract_server_timing_returns_none_when_absent() {
        assert!(extract_server_timing(&HeaderMap::new()).is_none());
    }

    #[test]
    fn extract_server_timing_returns_none_on_invalid_desc() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TIMING,
            HeaderValue::from_static("trace;desc=not-a-traceparent"),
        );
        assert!(extract_server_timing(&headers).is_none());
    }

    #[test]
    fn inject_then_extract_server_timing_roundtrip() {
        let tp: TraceParent = VALID_TP.parse().unwrap();
        let mut headers = HeaderMap::new();
        inject_server_timing(&tp, &mut headers);
        assert_eq!(extract_server_timing(&headers).unwrap(), tp);
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
