use super::*;

const MAX_ENTRIES: usize = 32;
const MAX_ENTRY_LEN: usize = 128;

/// A parsed W3C `tracestate` header value.
///
/// Holds an ordered list of up to 32 vendor key=value entries. The leftmost
/// entry belongs to the most recently updated vendor.
///
/// # Examples
///
/// ```
/// use trace_context_level3::TraceState;
///
/// let mut state: TraceState = "vendorname=opaquevalue".parse().unwrap();
/// state.insert("myvendor", "data").unwrap();
/// assert_eq!(state.get("myvendor"), Some("data"));
/// assert_eq!(state.to_string(), "myvendor=data,vendorname=opaquevalue");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceState(Vec<(String, String)>);

impl TraceState {
    /// Returns the value for `key`, or `None` if not present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Inserts or updates an entry, prepending it to the list.
    ///
    /// If a key already exists it is removed first, then the new entry is
    /// placed at the leftmost position. Fails if the key or value are
    /// syntactically invalid, or if the list is already full and no existing
    /// entry for the same key was found.
    ///
    /// # Errors
    ///
    /// Returns [`TraceStateError::InvalidKey`], [`TraceStateError::InvalidValue`],
    /// or [`TraceStateError::TooManyEntries`].
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), TraceStateError> {
        if !is_valid_key(key) {
            return Err(TraceStateError::invalid_key(key));
        }
        if !is_valid_value(value) {
            return Err(TraceStateError::invalid_value(value));
        }
        // Remove any existing entry first so a replacement never hits the cap.
        self.0.retain(|(k, _)| k != key);
        if self.0.len() >= MAX_ENTRIES {
            return Err(TraceStateError::TooManyEntries);
        }
        self.0.insert(0, (key.to_owned(), value.to_owned()));
        Ok(())
    }

    /// Removes the entry with `key`. Returns `true` if an entry was removed.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|(k, _)| k != key);
        self.0.len() < before
    }

    /// Iterates over `(key, value)` pairs in list order (leftmost first).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Returns the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Removes all entries.
    ///
    /// Call this on the `tracestate` accompanying a [`TraceParent::restart`]
    /// to avoid leaking upstream vendor data across trust boundaries.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Truncates the entry list so the serialised header value fits within
    /// `max_len` bytes, using the spec-defined two-step algorithm:
    ///
    /// 1. Remove entries whose serialised form (`key=value`) exceeds
    ///    [`MAX_ENTRY_LEN`] (128) bytes.
    /// 2. Remove rightmost entries (oldest / least recently updated) until
    ///    the header fits.
    pub fn truncate(&mut self, max_len: usize) {
        self.0
            .retain(|(k, v)| k.len() + 1 + v.len() <= MAX_ENTRY_LEN);
        while self.encoded_len() > max_len {
            self.0.pop();
        }
    }

    /// Parses a `tracestate` header value leniently, silently discarding any
    /// invalid or duplicate entries rather than failing the whole header.
    ///
    /// Use this when consuming headers received from an upstream system. Valid
    /// entries are preserved in order; the first occurrence of a duplicate key
    /// is kept and later occurrences are dropped. At most 32 entries are kept.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        let mut seen = std::collections::HashSet::new();
        let entries = s
            .split(',')
            .filter_map(|item| Self::validate_item(item.trim_matches([' ', '\t'])).ok())
            .filter(|&(key, _)| seen.insert(key))
            .take(MAX_ENTRIES)
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        Self(entries)
    }

    /// Returns the byte length of the serialised header value (`key=value`
    /// pairs joined by `,`), matching what [`Display`] would produce.
    fn encoded_len(&self) -> usize {
        if self.0.is_empty() {
            return 0;
        }
        self.0
            .iter()
            .map(|(k, v)| k.len() + 1 + v.len()) // +1 for '='
            .sum::<usize>()
            + (self.0.len() - 1) // n-1 ',' separators between entries
    }

    fn validate_item(item: &str) -> Result<(&str, &str), TraceStateError> {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| TraceStateError::invalid_key(item))?;
        if !is_valid_key(key) {
            return Err(TraceStateError::invalid_key(key));
        }
        if !is_valid_value(value) {
            return Err(TraceStateError::invalid_value(value));
        }
        Ok((key, value))
    }
}

impl fmt::Display for TraceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (key, value)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{key}={value}")?;
        }
        Ok(())
    }
}

impl str::FromStr for TraceState {
    type Err = TraceStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut seen = std::collections::HashSet::new();
        let entries = s
            .split(',')
            .filter_map(|m| {
                let m = m.trim_matches([' ', '\t']);
                (!m.is_empty()).then_some(m)
            })
            .map(|member| {
                let (key, value) = Self::validate_item(member)?;
                if !seen.insert(key) {
                    return Err(TraceStateError::duplicate_key(key));
                }
                Ok((key.to_owned(), value.to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if entries.len() > MAX_ENTRIES {
            return Err(TraceStateError::TooManyEntries);
        }
        Ok(Self(entries))
    }
}

/// Spec key grammar has two forms:
///
/// simple-key:       `(lcalpha / DIGIT) 0*255(keychar)` — max 256 chars, no `@`
/// multi-tenant-key: `tenant-id "@" system-id`
///   tenant-id:  `(lcalpha / DIGIT) 0*240(keychar)` — max 241 chars
///   system-id:  `lcalpha 0*13(lcalpha / DIGIT / "-")` — max 14 chars
///
/// `keychar = lcalpha / DIGIT / "_" / "-" / "*" / "/"` — note: no `@`
fn is_valid_key(key: &str) -> bool {
    match key.split_once('@') {
        None => is_valid_simple_key(key),
        Some((_, system)) if system.contains('@') => false,
        Some((tenant, system)) => is_valid_tenant_id(tenant) && is_valid_system_id(system),
    }
}

fn is_valid_simple_key(key: &str) -> bool {
    let b = key.as_bytes();
    matches!(b, [first, rest @ ..] if b.len() <= 256
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest.iter().all(|&c| is_keychar(c)))
}

fn is_valid_tenant_id(s: &str) -> bool {
    let b = s.as_bytes();
    matches!(b, [first, rest @ ..] if b.len() <= 241
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest.iter().all(|&c| is_keychar(c)))
}

fn is_valid_system_id(s: &str) -> bool {
    let b = s.as_bytes();
    matches!(b, [first, rest @ ..] if b.len() <= 14
        && first.is_ascii_lowercase()
        && rest.iter().all(|&c| is_system_id_char(c)))
}

fn is_keychar(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'*' | b'/')
}

fn is_system_id_char(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

/// `value = 0*255(chr) nblk-chr`  (length 1–256)
/// `chr = %x20 / nblk-chr`
/// `nblk-chr = %x21-2B / %x2D-3C / %x3E-7E`  (excludes `,` 0x2C and `=` 0x3D)
fn is_valid_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(&last) = bytes.last() else {
        return false; // empty
    };
    bytes.len() <= 256 && is_nblk_chr(last) && bytes.iter().all(|&b| is_chr(b))
}

fn is_chr(b: u8) -> bool {
    matches!(b, 0x20..=0x2B | 0x2D..=0x3C | 0x3E..=0x7E)
}

fn is_nblk_chr(b: u8) -> bool {
    matches!(b, 0x21..=0x2B | 0x2D..=0x3C | 0x3E..=0x7E)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let state: TraceState = "foo=bar".parse().unwrap();
        assert_eq!(state.get("foo"), Some("bar"));
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn parse_multiple_entries() {
        let state: TraceState = "foo=bar,baz=qux".parse().unwrap();
        assert_eq!(state.get("foo"), Some("bar"));
        assert_eq!(state.get("baz"), Some("qux"));
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_with_ows() {
        let state: TraceState = "foo=bar , baz=qux".parse().unwrap();
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_empty_members_skipped() {
        let state: TraceState = "foo=bar,,baz=qux".parse().unwrap();
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_key_starts_with_digit() {
        let state: TraceState = "1vendor=val".parse().unwrap();
        assert_eq!(state.get("1vendor"), Some("val"));
    }

    #[test]
    fn parse_multi_tenant_key() {
        let state: TraceState = "tenant@system=val".parse().unwrap();
        assert_eq!(state.get("tenant@system"), Some("val"));
    }

    #[test]
    fn parse_multi_tenant_key_digit_tenant() {
        let state: TraceState = "1tenant@sys=v".parse().unwrap();
        assert_eq!(state.get("1tenant@sys"), Some("v"));
    }

    #[test]
    fn rejects_multi_tenant_key_double_at() {
        assert!("a@@b=v".parse::<TraceState>().is_err());
        assert!("a@b@c=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_empty_tenant() {
        assert!("@system=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_empty_system() {
        assert!("tenant@=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_system_starts_with_digit() {
        assert!("tenant@1sys=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_system_with_underscore() {
        // system-id allows only lcalpha / DIGIT / "-", not "_"
        assert!("tenant@sys_id=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_tenant_id_too_long() {
        let tenant = "a".repeat(242);
        assert!(format!("{tenant}@sys=v").parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_multi_tenant_key_system_id_too_long() {
        let system = "a".repeat(15);
        assert!(format!("tenant@{system}=v").parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_simple_key_with_at() {
        // bare @ without a valid multi-tenant structure
        assert!("@=v".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_key_uppercase() {
        assert!("Vendor=val".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_key_starts_with_dash() {
        assert!("-vendor=val".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_key_too_long() {
        let key = "a".repeat(257);
        assert!(format!("{key}=val").parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_value_with_comma() {
        assert!("foo=bar,baz".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_value_empty() {
        assert!("foo=".parse::<TraceState>().is_err());
    }

    #[test]
    fn rejects_too_many_entries() {
        let s = (0..33)
            .map(|i| format!("k{i}=v"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(matches!(
            s.parse::<TraceState>(),
            Err(TraceStateError::TooManyEntries)
        ));
    }

    #[test]
    fn insert_rejects_value_trailing_space() {
        let err = TraceState::default().insert("foo", "bar ").unwrap_err();
        assert!(matches!(err, TraceStateError::InvalidValue(_)));
    }

    #[test]
    fn insert_prepends() {
        let mut state: TraceState = "existing=val".parse().unwrap();
        state.insert("new", "entry").unwrap();
        let mut iter = state.iter();
        assert_eq!(iter.next(), Some(("new", "entry")));
        assert_eq!(iter.next(), Some(("existing", "val")));
    }

    #[test]
    fn insert_replaces_and_prepends() {
        let mut state: TraceState = "foo=old,bar=baz".parse().unwrap();
        state.insert("foo", "new").unwrap();
        let mut iter = state.iter();
        assert_eq!(iter.next(), Some(("foo", "new")));
        assert_eq!(iter.next(), Some(("bar", "baz")));
        assert_eq!(iter.next(), None);
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn insert_replace_does_not_hit_cap() {
        let mut state = TraceState::default();
        for i in 0..MAX_ENTRIES {
            state.insert(&format!("k{i}"), "v").unwrap();
        }
        assert_eq!(state.len(), MAX_ENTRIES);
        // Replacing an existing key must succeed even at the cap.
        state.insert("k0", "updated").unwrap();
        assert_eq!(state.len(), MAX_ENTRIES);
        assert_eq!(state.get("k0"), Some("updated"));
    }

    #[test]
    fn insert_at_cap_new_key_fails() {
        let mut state = TraceState::default();
        for i in 0..MAX_ENTRIES {
            state.insert(&format!("k{i}"), "v").unwrap();
        }
        assert!(matches!(
            state.insert("new", "v"),
            Err(TraceStateError::TooManyEntries)
        ));
    }

    #[test]
    fn remove_existing() {
        let mut state: TraceState = "foo=bar,baz=qux".parse().unwrap();
        assert!(state.remove("foo"));
        assert_eq!(state.get("foo"), None);
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn remove_missing_returns_false() {
        let mut state: TraceState = "foo=bar".parse().unwrap();
        assert!(!state.remove("missing"));
    }

    #[test]
    fn get_missing_returns_none() {
        let state: TraceState = "foo=bar".parse().unwrap();
        assert_eq!(state.get("other"), None);
    }

    #[test]
    fn display_roundtrip() {
        let original = "foo=bar,baz=qux";
        let state: TraceState = original.parse().unwrap();
        assert_eq!(state.to_string(), original);
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut state: TraceState = "foo=bar,baz=qux".parse().unwrap();
        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.to_string(), "");
    }

    #[test]
    fn default_is_empty() {
        let state = TraceState::default();
        assert!(state.is_empty());
        assert_eq!(state.to_string(), "");
    }

    // --- duplicate key detection ---

    #[test]
    fn rejects_duplicate_keys_in_strict_parse() {
        assert!(matches!(
            "foo=a,foo=b".parse::<TraceState>(),
            Err(TraceStateError::DuplicateKey(_))
        ));
    }

    #[test]
    fn insert_replace_does_not_produce_duplicate() {
        let mut state: TraceState = "foo=a".parse().unwrap();
        state.insert("foo", "b").unwrap();
        assert_eq!(state.len(), 1);
        assert_eq!(state.get("foo"), Some("b"));
    }

    // --- parse_lenient ---

    #[test]
    fn parse_lenient_discards_invalid_key() {
        let state = TraceState::parse_lenient("valid=ok,!!!bad!!!=v,other=2");
        assert_eq!(state.get("valid"), Some("ok"));
        assert_eq!(state.get("other"), Some("2"));
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_lenient_discards_entry_without_eq() {
        let state = TraceState::parse_lenient("a=1,noequalssign,b=2");
        assert_eq!(state.get("a"), Some("1"));
        assert_eq!(state.get("b"), Some("2"));
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_lenient_keeps_first_on_duplicate() {
        let state = TraceState::parse_lenient("foo=first,bar=x,foo=second");
        assert_eq!(state.get("foo"), Some("first"));
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn parse_lenient_empty_string() {
        assert!(TraceState::parse_lenient("").is_empty());
    }

    // --- truncate ---

    #[test]
    fn truncate_noop_when_already_fits() {
        let mut state: TraceState = "foo=bar,baz=qux".parse().unwrap();
        state.truncate(100);
        assert_eq!(state.len(), 2);
        assert_eq!(state.get("foo"), Some("bar"));
        assert_eq!(state.get("baz"), Some("qux"));
    }

    #[test]
    fn truncate_keeps_entry_at_exactly_128_bytes() {
        // "k=" + 126 x's = 128 bytes exactly — must be kept
        let mut state = TraceState::default();
        state.insert("k", &"x".repeat(126)).unwrap();
        state.truncate(usize::MAX);
        assert!(state.get("k").is_some());
    }

    #[test]
    fn truncate_removes_entry_over_128_bytes() {
        // "k=" + 127 x's = 129 bytes — must be removed in step 1
        let mut state = TraceState::default();
        state.insert("small", "value").unwrap();
        state.insert("k", &"x".repeat(127)).unwrap();
        state.truncate(usize::MAX);
        assert_eq!(state.get("k"), None);
        assert_eq!(state.get("small"), Some("value"));
    }

    #[test]
    fn truncate_removes_rightmost_to_fit() {
        // "a=1,b=2,c=3" = 11 bytes; truncate to 8 should drop "c=3"
        let mut state: TraceState = "a=1,b=2,c=3".parse().unwrap();
        state.truncate(8);
        assert_eq!(state.get("a"), Some("1"));
        assert_eq!(state.get("b"), Some("2"));
        assert_eq!(state.get("c"), None);
        assert!(state.encoded_len() <= 8);
    }

    #[test]
    fn truncate_removes_multiple_rightmost() {
        // "a=1,b=2,c=3" = 11 bytes; truncate to 3 should leave only "a=1"
        let mut state: TraceState = "a=1,b=2,c=3".parse().unwrap();
        state.truncate(3);
        assert_eq!(state.len(), 1);
        assert_eq!(state.get("a"), Some("1"));
    }

    #[test]
    fn truncate_clears_all_when_max_len_zero() {
        let mut state: TraceState = "a=1,b=2".parse().unwrap();
        state.truncate(0);
        assert!(state.is_empty());
    }

    #[test]
    fn truncate_step1_then_step2() {
        // insert() prepends, so insertion order is: a (oldest/rightmost), b, big (newest/leftmost).
        let mut state = TraceState::default();
        state.insert("a", "1").unwrap();
        state.insert("b", "2").unwrap();
        state.insert("big", &"x".repeat(127)).unwrap(); // 3+1+127 = 131 > 128
        // Step 1: "big" removed (>128) → internal: [b=2, a=1] = 7 bytes
        // Step 2: max_len=4 → pop rightmost "a=1" → [b=2] = 3 bytes ≤ 4
        state.truncate(4);
        assert_eq!(state.get("big"), None); // removed by step 1
        assert_eq!(state.get("a"), None); // oldest entry, removed by step 2
        assert_eq!(state.get("b"), Some("2")); // newest remaining, kept
    }
}
