use core::str::FromStr;
use revm_primitives::hardfork::SpecId;
pub use revm_primitives::hardfork::UnknownHardfork;

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
    FEYNMAN = 6,
    GALILEO = 7,
    TSUKI = 8,
    /// The default is the latest spec. Execution must select the spec from fork activation
    /// instead of relying on the default.
    #[default]
    TSUKI_V2 = 9,
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
            Self::SHANGHAI |
            Self::BERNOULLI |
            Self::CURIE |
            Self::DARWIN |
            Self::EUCLID |
            Self::FEYNMAN |
            Self::GALILEO |
            Self::TSUKI |
            Self::TSUKI_V2 => SpecId::SHANGHAI,
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
    pub const TSUKI: &str = "tsuki";
    pub const TSUKI_V2: &str = "tsukiV2";
}

impl FromStr for ScrollSpecId {
    type Err = UnknownHardfork;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            name::SHANGHAI => Ok(Self::SHANGHAI),
            name::BERNOULLI => Ok(Self::BERNOULLI),
            name::CURIE => Ok(Self::CURIE),
            name::DARWIN => Ok(Self::DARWIN),
            name::EUCLID => Ok(Self::EUCLID),
            name::FEYNMAN => Ok(Self::FEYNMAN),
            name::GALILEO => Ok(Self::GALILEO),
            name::TSUKI => Ok(Self::TSUKI),
            name::TSUKI_V2 => Ok(Self::TSUKI_V2),
            _ => Err(UnknownHardfork),
        }
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
            ScrollSpecId::TSUKI => name::TSUKI,
            ScrollSpecId::TSUKI_V2 => name::TSUKI_V2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_names_round_trip() {
        for spec in [
            ScrollSpecId::SHANGHAI,
            ScrollSpecId::BERNOULLI,
            ScrollSpecId::CURIE,
            ScrollSpecId::DARWIN,
            ScrollSpecId::EUCLID,
            ScrollSpecId::FEYNMAN,
            ScrollSpecId::GALILEO,
            ScrollSpecId::TSUKI,
            ScrollSpecId::TSUKI_V2,
        ] {
            let name: &'static str = spec.into();
            assert_eq!(name.parse::<ScrollSpecId>(), Ok(spec));
            assert_eq!(ScrollSpecId::try_from_u8(spec as u8), Some(spec));
        }
    }

    #[test]
    fn unknown_spec_name_is_an_error() {
        for name in ["", "prague", "Euclid", "tsukiv2", "tsuki_v2", " tsuki"] {
            assert_eq!(name.parse::<ScrollSpecId>(), Err(UnknownHardfork), "{name:?}");
        }
    }

    #[test]
    fn default_is_latest_spec() {
        assert_eq!(ScrollSpecId::default(), ScrollSpecId::TSUKI_V2);
        assert_eq!(ScrollSpecId::try_from_u8(ScrollSpecId::default() as u8 + 1), None);
    }

    #[test]
    fn tsuki_v2_follows_tsuki() {
        let name: &'static str = ScrollSpecId::TSUKI_V2.into();
        assert_eq!(name, "tsukiV2");
        assert_eq!(ScrollSpecId::TSUKI_V2 as u8, 9);
        assert_eq!(ScrollSpecId::try_from_u8(10), None);
        assert_eq!(SpecId::from(ScrollSpecId::TSUKI_V2), SpecId::SHANGHAI);
        assert!(ScrollSpecId::TSUKI_V2.is_enabled_in(ScrollSpecId::TSUKI));
        assert!(!ScrollSpecId::TSUKI.is_enabled_in(ScrollSpecId::TSUKI_V2));
    }
}
