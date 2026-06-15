use super::precompile_not_implemented;

use revm::{precompile::hash, primitives::Address};

pub mod sha256 {
    use super::*;
    use revm::precompile::{u64_to_address, Precompile, PrecompileId};

    /// SHA-256 precompile address
    pub const ADDRESS: Address = u64_to_address(2);

    /// The SHA256 precompile is not implemented in the Shanghai hardfork.
    pub const SHANGHAI: Precompile = precompile_not_implemented(PrecompileId::Sha256, ADDRESS);

    /// The bernoulli SHA256 precompile implementation with address.
    pub const BERNOULLI: Precompile =
        Precompile::new(PrecompileId::Sha256, ADDRESS, hash::sha256_run);
}

pub mod ripemd160 {
    use super::*;
    use revm::precompile::{u64_to_address, Precompile, PrecompileError, PrecompileId};

    /// The RIPEMD160 precompile address.
    pub const ADDRESS: Address = u64_to_address(3);

    /// The shanghai RIPEMD160 precompile is not implemented in the Shanghai hardfork.
    ///
    /// This precompile is not implemented and will return `PrecompileError::Other("Precompile not
    /// implemented".into())`.
    pub const SHANGHAI: Precompile = precompile_not_implemented(PrecompileId::Ripemd160, ADDRESS);

    /// The maximum length of the input for the RIPEMD160 precompile in the GALDOGEOS hardfork.
    pub const GALDOGEOS_LEN_LIMIT: usize = 32;

    /// The GALDOGEOS RIPEMD160 is enabled.
    pub const GALDOGEOS: Precompile =
        Precompile::new(PrecompileId::Ripemd160, ADDRESS, |input, gas_limit| {
            if input.len() > GALDOGEOS_LEN_LIMIT {
                return Err(PrecompileError::Other(
                    "Ripemd160InputOverflow: ripemd160 input overflow".into(),
                ));
            }
            hash::ripemd160_run(input, gas_limit)
        });
}
