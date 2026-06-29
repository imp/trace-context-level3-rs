use super::*;

/// A 16-byte trace identifier, serialized as 32 lowercase hex characters.
///
/// The all-zeros value is forbidden by the W3C Trace Context specification.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Constructs a `TraceId` from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TraceParentError::ZeroTraceId`] if all bytes are zero.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, TraceParentError> {
        if bytes == [0u8; 16] {
            return Err(TraceParentError::ZeroTraceId);
        }
        Ok(Self(bytes))
    }

    /// Returns the underlying bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraceId({self})")
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::LowerHex for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl str::FromStr for TraceId {
    type Err = TraceParentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 {
            return Err(TraceParentError::InvalidHex);
        }
        let raw = parse_hex_bytes::<16>(s.as_bytes())?;
        Self::from_bytes(raw)
    }
}

/// An 8-byte parent span identifier, serialized as 16 lowercase hex characters.
///
/// The all-zeros value is forbidden by the W3C Trace Context specification.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParentId([u8; 8]);

impl ParentId {
    /// Constructs a `ParentId` from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TraceParentError::ZeroParentId`] if all bytes are zero.
    pub fn from_bytes(bytes: [u8; 8]) -> Result<Self, TraceParentError> {
        if bytes == [0u8; 8] {
            return Err(TraceParentError::ZeroParentId);
        }
        Ok(Self(bytes))
    }

    /// Returns the underlying bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Debug for ParentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParentId({self})")
    }
}

impl fmt::Display for ParentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::LowerHex for ParentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl str::FromStr for ParentId {
    type Err = TraceParentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 16 {
            return Err(TraceParentError::InvalidHex);
        }
        let raw = parse_hex_bytes::<8>(s.as_bytes())?;
        Self::from_bytes(raw)
    }
}

/// The trace-flags byte: an 8-bit field where each bit has independent meaning.
///
/// Bits not defined here are reserved and MUST be set to zero on outgoing requests.
///
/// # Defined flags
///
/// | Bit | Mask   | Name            | Description                                       |
/// |-----|--------|-----------------|---------------------------------------------------|
/// | 0   | `0x01` | `SAMPLED`        | Caller may have recorded trace data               |
/// | 1   | `0x02` | `RANDOM_TRACE_ID`| Rightmost 7 bytes of `trace-id` are random        |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TraceFlags(u8);

impl TraceFlags {
    /// The sampled flag (bit 0): the caller may have recorded trace data.
    pub const SAMPLED: Self = Self(0x01);

    /// The random-trace-id flag (bit 1, added in Level 2): at least the
    /// rightmost 7 bytes of `trace-id` were randomly generated.
    ///
    /// When forwarding a `traceparent` with the same `trace-id`, this flag
    /// MUST be preserved as-is.
    pub const RANDOM_TRACE_ID: Self = Self(0x02);

    /// Constructs `TraceFlags` from a raw byte value.
    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw byte value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns `true` if the sampled flag (bit 0) is set.
    #[inline]
    #[must_use]
    pub const fn is_sampled(self) -> bool {
        self.0 & Self::SAMPLED.as_u8() != 0
    }

    /// Returns `true` if the random-trace-id flag (bit 1) is set.
    #[inline]
    #[must_use]
    pub const fn is_random_trace_id(self) -> bool {
        self.0 & Self::RANDOM_TRACE_ID.as_u8() != 0
    }

    /// Returns a copy with the sampled flag set or cleared.
    #[inline]
    #[must_use]
    pub const fn with_sampled(self, sampled: bool) -> Self {
        if sampled {
            Self(self.0 | Self::SAMPLED.as_u8())
        } else {
            Self(self.0 & !Self::SAMPLED.as_u8())
        }
    }

    /// Returns a copy with the random-trace-id flag set or cleared.
    #[inline]
    #[must_use]
    pub const fn with_random_trace_id(self, random: bool) -> Self {
        if random {
            Self(self.0 | Self::RANDOM_TRACE_ID.as_u8())
        } else {
            Self(self.0 & !Self::RANDOM_TRACE_ID.as_u8())
        }
    }

    /// Returns a copy with all reserved bits (2–7) cleared, as required when
    /// generating or forwarding a `traceparent`.
    #[inline]
    #[must_use]
    pub const fn with_reserved_cleared(self) -> Self {
        Self(self.0 & (Self::SAMPLED.as_u8() | Self::RANDOM_TRACE_ID.as_u8()))
    }
}

impl ops::BitOr for TraceFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl ops::BitOrAssign for TraceFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl ops::BitAnd for TraceFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl ops::BitAndAssign for TraceFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl fmt::Display for TraceFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}", self.0)
    }
}

/// A parsed W3C `traceparent` header value.
///
/// Wire format: `{version}-{trace-id}-{parent-id}-{trace-flags}`
///
/// # Examples
///
/// ```
/// use trace_context_level3::TraceParent;
///
/// let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
/// let tp: TraceParent = header.parse().unwrap();
/// assert!(tp.is_sampled());
/// assert_eq!(tp.to_string(), header);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraceParent {
    /// Version byte. Currently `0x00`; `0xff` is always invalid.
    pub version: u8,
    /// 128-bit trace identifier.
    pub trace_id: TraceId,
    /// 64-bit parent span identifier representing the current operation.
    pub parent_id: ParentId,
    /// Trace flags bitfield.
    pub trace_flags: TraceFlags,
}

impl TraceParent {
    /// The only version currently defined by the spec.
    pub const VERSION_0: u8 = 0x00;

    /// Version byte reserved by the spec; always rejected on input.
    const INVALID_VERSION: u8 = 0xFF;

    /// Creates a version-`00` `TraceParent`.
    #[must_use]
    pub fn new(trace_id: TraceId, parent_id: ParentId, trace_flags: TraceFlags) -> Self {
        Self {
            version: Self::VERSION_0,
            trace_id,
            parent_id,
            trace_flags,
        }
    }

    /// Returns `true` if the sampled flag is set.
    #[inline]
    #[must_use]
    pub fn is_sampled(self) -> bool {
        self.trace_flags.is_sampled()
    }

    /// Returns `true` if the random-trace-id flag is set.
    #[inline]
    #[must_use]
    pub fn is_random_trace_id(self) -> bool {
        self.trace_flags.is_random_trace_id()
    }
}

impl fmt::Display for TraceParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}-{}-{}-{}",
            self.version, self.trace_id, self.parent_id, self.trace_flags,
        )
    }
}

impl str::FromStr for TraceParent {
    type Err = TraceParentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();

        // Spec §3.2: if shorter than 55 chars, MUST restart the trace.
        if bytes.len() < 55 {
            return Err(TraceParentError::TooShort(bytes.len()));
        }

        // version: positions 0–1
        let version = parse_hex_bytes::<1>(&bytes[0..2])?[0];
        if version == Self::INVALID_VERSION {
            return Err(TraceParentError::InvalidVersion);
        }

        // separator at position 2
        if bytes[2] != b'-' {
            return Err(TraceParentError::MissingSeparator);
        }

        // trace-id: positions 3–34 (32 hex chars → 16 bytes)
        let trace_id = TraceId::from_bytes(parse_hex_bytes::<16>(&bytes[3..35])?)?;

        // separator at position 35
        if bytes[35] != b'-' {
            return Err(TraceParentError::MissingSeparator);
        }

        // parent-id: positions 36–51 (16 hex chars → 8 bytes)
        let parent_id = ParentId::from_bytes(parse_hex_bytes::<8>(&bytes[36..52])?)?;

        // separator at position 52
        if bytes[52] != b'-' {
            return Err(TraceParentError::MissingSeparator);
        }

        // trace-flags: positions 53–54 (2 hex chars → 1 byte)
        let trace_flags = TraceFlags::from_u8(parse_hex_bytes::<1>(&bytes[53..55])?[0]);

        // Version 00: must be exactly 55 characters; no trailing data.
        // Future versions (01–FE): trailing content after position 54 is ignored.
        if version == Self::VERSION_0 && bytes.len() != 55 {
            return Err(TraceParentError::TrailingData(bytes.len()));
        }

        Ok(Self {
            version,
            trace_id,
            parent_id,
            trace_flags,
        })
    }
}

/// Parses exactly `N` bytes from a `2 * N`-character lowercase-hex byte slice.
pub(crate) fn parse_hex_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N], TraceParentError> {
    debug_assert_eq!(bytes.len(), N * 2);
    let mut result = [0u8; N];
    for i in 0..N {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        result[i] = (hi << 4) | lo;
    }
    Ok(result)
}

fn hex_nibble(b: u8) -> Result<u8, TraceParentError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(TraceParentError::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn parse_valid_sampled() {
        let tp: TraceParent = VALID.parse().unwrap();
        assert_eq!(tp.version, TraceParent::VERSION_0);
        assert_eq!(tp.to_string(), VALID);
        assert!(tp.is_sampled());
        assert!(!tp.is_random_trace_id());
    }

    #[test]
    fn parse_valid_not_sampled() {
        let s = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00";
        let tp: TraceParent = s.parse().unwrap();
        assert!(!tp.is_sampled());
        assert_eq!(tp.to_string(), s);
    }

    #[test]
    fn parse_random_trace_id_flag() {
        let s = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-03";
        let tp: TraceParent = s.parse().unwrap();
        assert!(tp.is_sampled());
        assert!(tp.is_random_trace_id());
        assert_eq!(tp.to_string(), s);
    }

    #[test]
    fn rejects_too_short() {
        let err = "00-4bf92f3577b34da6a-00f067aa0ba902b7-01"
            .parse::<TraceParent>()
            .unwrap_err();
        assert!(matches!(err, TraceParentError::TooShort(_)));
    }

    #[test]
    fn rejects_version_ff() {
        let s = "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        assert_eq!(
            s.parse::<TraceParent>().unwrap_err(),
            TraceParentError::InvalidVersion
        );
    }

    #[test]
    fn rejects_uppercase_hex() {
        let s = "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01";
        assert!(matches!(
            s.parse::<TraceParent>(),
            Err(TraceParentError::InvalidHex)
        ));
    }

    #[test]
    fn rejects_zero_trace_id() {
        let s = "00-00000000000000000000000000000000-00f067aa0ba902b7-01";
        assert_eq!(
            s.parse::<TraceParent>().unwrap_err(),
            TraceParentError::ZeroTraceId
        );
    }

    #[test]
    fn rejects_zero_parent_id() {
        let s = "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01";
        assert_eq!(
            s.parse::<TraceParent>().unwrap_err(),
            TraceParentError::ZeroParentId
        );
    }

    #[test]
    fn rejects_trailing_data_for_v00() {
        let s = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra";
        assert!(matches!(
            s.parse::<TraceParent>(),
            Err(TraceParentError::TrailingData(_))
        ));
    }

    #[test]
    fn future_version_ignores_trailing() {
        // version 01, longer than 55 chars — spec says ignore trailing data
        let s = "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-unknownfield";
        let tp: TraceParent = s.parse().unwrap();
        assert_eq!(tp.version, 0x01);
        assert!(tp.is_sampled());
    }

    #[test]
    fn roundtrip_display_parse() {
        let tp: TraceParent = VALID.parse().unwrap();
        assert_eq!(tp.to_string().parse::<TraceParent>().unwrap(), tp);
    }

    #[test]
    fn trace_flags_bitor() {
        let flags = TraceFlags::SAMPLED | TraceFlags::RANDOM_TRACE_ID;
        assert!(flags.is_sampled());
        assert!(flags.is_random_trace_id());
    }

    #[test]
    fn trace_flags_bitand_mask() {
        let flags = TraceFlags::SAMPLED | TraceFlags::RANDOM_TRACE_ID;
        assert_eq!(flags & TraceFlags::SAMPLED, TraceFlags::SAMPLED);
        assert_eq!(
            flags & TraceFlags::RANDOM_TRACE_ID,
            TraceFlags::RANDOM_TRACE_ID
        );
    }

    #[test]
    fn trace_flags_assign_ops() {
        let mut flags = TraceFlags::SAMPLED;
        flags |= TraceFlags::RANDOM_TRACE_ID;
        assert!(flags.is_random_trace_id());
        flags &= TraceFlags::SAMPLED;
        assert!(!flags.is_random_trace_id());
        assert!(flags.is_sampled());
    }

    #[test]
    fn trace_flags_reserved_cleared() {
        let flags = TraceFlags::from_u8(0xFF).with_reserved_cleared();
        assert_eq!(
            flags.as_u8(),
            TraceFlags::SAMPLED.as_u8() | TraceFlags::RANDOM_TRACE_ID.as_u8()
        );
    }

    #[test]
    fn trace_id_from_str() {
        let id: TraceId = "4bf92f3577b34da6a3ce929d0e0e4736".parse().unwrap();
        assert_eq!(id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn parent_id_from_str() {
        let id: ParentId = "00f067aa0ba902b7".parse().unwrap();
        assert_eq!(id.to_string(), "00f067aa0ba902b7");
    }
}
