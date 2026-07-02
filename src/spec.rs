use revm_primitives::hardfork::SpecId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseScrollSpecIdError;

impl core::fmt::Display for ParseScrollSpecIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("unknown Scroll hardfork name")
    }
}

impl core::error::Error for ParseScrollSpecIdError {}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, enumn::N)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(non_camel_case_types)]
pub enum ScrollSpecId {
    SHANGHAI = 1,
    BERNOULLI = 2,
    CURIE = 3,
    DARWIN = 4,
    EUCLID = 5,
    #[default]
    FEYNMAN = 6,
    GALILEO = 7,
}

impl ScrollSpecId {
    /// Returns the `ScrollSpecId` for the given `u8`.
    #[inline]
    pub fn try_from_u8(spec_id: u8) -> Option<Self> {
        Self::n(spec_id)
    }

    /// Returns `true` if the given specification ID is enabled in this spec.
    #[inline]
    pub const fn is_enabled_in(self, other: Self) -> bool {
        Self::enabled(self, other)
    }

    /// Returns `true` if the provided specification ID is enabled in the other spec.
    #[inline]
    pub const fn enabled(our: Self, other: Self) -> bool {
        our as u8 >= other as u8
    }

    /// Converts the `ScrollSpecId` to a `SpecId`.
    const fn into_eth_spec_id(self) -> SpecId {
        match self {
            Self::SHANGHAI
            | Self::BERNOULLI
            | Self::CURIE
            | Self::DARWIN
            | Self::EUCLID
            | Self::FEYNMAN
            | Self::GALILEO => SpecId::SHANGHAI,
        }
    }
}

impl From<ScrollSpecId> for SpecId {
    fn from(spec_id: ScrollSpecId) -> Self {
        spec_id.into_eth_spec_id()
    }
}

/// String identifiers for the Scroll hardforks.
pub mod name {
    // Re-export the Ethereum hardforks.
    pub use revm_primitives::hardfork::name::{LATEST, SHANGHAI};

    pub const BERNOULLI: &str = "bernoulli";
    pub const CURIE: &str = "curie";
    pub const DARWIN: &str = "darwin";
    pub const EUCLID: &str = "euclid";
    pub const FEYNMAN: &str = "feynman";
    pub const GALILEO: &str = "galileo";
}

impl core::str::FromStr for ScrollSpecId {
    type Err = ParseScrollSpecIdError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            name::SHANGHAI => Ok(Self::SHANGHAI),
            name::BERNOULLI => Ok(Self::BERNOULLI),
            name::CURIE => Ok(Self::CURIE),
            name::DARWIN => Ok(Self::DARWIN),
            name::EUCLID => Ok(Self::EUCLID),
            name::FEYNMAN => Ok(Self::FEYNMAN),
            name::GALILEO => Ok(Self::GALILEO),
            _ => Err(ParseScrollSpecIdError),
        }
    }
}

impl TryFrom<&str> for ScrollSpecId {
    type Error = ParseScrollSpecIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ScrollSpecId> for &'static str {
    fn from(value: ScrollSpecId) -> Self {
        match value {
            ScrollSpecId::SHANGHAI => name::SHANGHAI,
            ScrollSpecId::BERNOULLI => name::BERNOULLI,
            ScrollSpecId::CURIE => name::CURIE,
            ScrollSpecId::DARWIN => name::DARWIN,
            ScrollSpecId::EUCLID => name::EUCLID,
            ScrollSpecId::FEYNMAN => name::FEYNMAN,
            ScrollSpecId::GALILEO => name::GALILEO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_is_feynman() {
        assert_eq!(ScrollSpecId::default(), ScrollSpecId::FEYNMAN);
    }

    #[test]
    fn parses_known_spec_names() {
        for (name, expected) in [
            (name::SHANGHAI, ScrollSpecId::SHANGHAI),
            (name::BERNOULLI, ScrollSpecId::BERNOULLI),
            (name::CURIE, ScrollSpecId::CURIE),
            (name::DARWIN, ScrollSpecId::DARWIN),
            (name::EUCLID, ScrollSpecId::EUCLID),
            (name::FEYNMAN, ScrollSpecId::FEYNMAN),
            (name::GALILEO, ScrollSpecId::GALILEO),
        ] {
            assert_eq!(name.parse::<ScrollSpecId>(), Ok(expected));
            assert_eq!(ScrollSpecId::try_from(name), Ok(expected));
        }
    }

    #[test]
    fn rejects_unknown_spec_names() {
        assert_eq!("feynmann".parse::<ScrollSpecId>(), Err(ParseScrollSpecIdError));
        assert_eq!(ScrollSpecId::try_from(""), Err(ParseScrollSpecIdError));
    }

    #[test]
    fn scroll_spec_order_matches_activation_order() {
        assert!(ScrollSpecId::SHANGHAI < ScrollSpecId::BERNOULLI);
        assert!(ScrollSpecId::BERNOULLI < ScrollSpecId::CURIE);
        assert!(ScrollSpecId::CURIE < ScrollSpecId::DARWIN);
        assert!(ScrollSpecId::DARWIN < ScrollSpecId::EUCLID);
        assert!(ScrollSpecId::EUCLID < ScrollSpecId::FEYNMAN);
        assert!(ScrollSpecId::FEYNMAN < ScrollSpecId::GALILEO);
    }

    #[test]
    fn all_scroll_specs_use_shanghai_eth_base_spec() {
        for spec in [
            ScrollSpecId::SHANGHAI,
            ScrollSpecId::BERNOULLI,
            ScrollSpecId::CURIE,
            ScrollSpecId::DARWIN,
            ScrollSpecId::EUCLID,
            ScrollSpecId::FEYNMAN,
            ScrollSpecId::GALILEO,
        ] {
            assert_eq!(SpecId::from(spec), SpecId::SHANGHAI);
        }
    }
}
