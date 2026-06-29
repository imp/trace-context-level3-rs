use super::*;

const MAX_ENTRIES: usize = 32;

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
            return Err(TraceStateError::InvalidKey(key.to_owned()));
        }
        if !is_valid_value(value) {
            return Err(TraceStateError::InvalidValue(value.to_owned()));
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
        let mut entries: Vec<(String, String)> = Vec::new();
        for member in s.split(',') {
            // OWS = optional whitespace (SP / HTAB) per RFC 9110
            let member = member.trim_matches(|c| c == ' ' || c == '\t');
            if member.is_empty() {
                continue; // OWS-only member; valid but carries no data
            }
            let eq = member
                .find('=')
                .ok_or_else(|| TraceStateError::InvalidKey(member.to_owned()))?;
            let key = &member[..eq];
            let value = &member[eq + 1..];
            if !is_valid_key(key) {
                return Err(TraceStateError::InvalidKey(key.to_owned()));
            }
            if !is_valid_value(value) {
                return Err(TraceStateError::InvalidValue(value.to_owned()));
            }
            if entries.len() >= MAX_ENTRIES {
                return Err(TraceStateError::TooManyEntries);
            }
            entries.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self(entries))
    }
}

/// Level 2/3 key grammar:
/// `key = ( lcalpha / DIGIT ) 0*255( keychar )`
/// `keychar = lcalpha / DIGIT / "_" / "-" / "*" / "/" / "@"`
fn is_valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    match bytes {
        [] => false,
        [first, rest @ ..] if bytes.len() <= 256 => {
            (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest.iter().all(|&b| is_keychar(b))
        }
        _ => false,
    }
}

fn is_keychar(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'*' | b'/' | b'@')
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
    fn parse_key_with_at_sign() {
        let state: TraceState = "tenant@system=val".parse().unwrap();
        assert_eq!(state.get("tenant@system"), Some("val"));
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
        let mut state = TraceState::default();
        assert!(matches!(
            state.insert("foo", "bar "),
            Err(TraceStateError::InvalidValue(_))
        ));
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
}
