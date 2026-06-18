use super::precompile_not_implemented;

use revm::{precompile::hash, primitives::Address};

pub mod sha256 {
    use super::*;
    use revm::precompile::{
        call_eth_precompile, u64_to_address, Precompile, PrecompileId, PrecompileResult,
    };

    /// SHA-256 precompile address
    pub const ADDRESS: Address = u64_to_address(2);

    /// The SHA256 precompile is not implemented in the Shanghai hardfork.
    pub const SHANGHAI: Precompile = precompile_not_implemented(PrecompileId::Sha256, ADDRESS);

    fn bernoulli_run(input: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
        Ok(call_eth_precompile(hash::sha256_run, input, gas_limit, reservoir))
    }

    /// The bernoulli SHA256 precompile implementation with address.
    pub const BERNOULLI: Precompile = Precompile::new(PrecompileId::Sha256, ADDRESS, bernoulli_run);
}

pub mod ripemd160 {
    use super::*;
    use revm::precompile::{
        call_eth_precompile, u64_to_address, Precompile, PrecompileHalt, PrecompileId,
        PrecompileOutput, PrecompileResult,
    };

    /// The RIPEMD160 precompile address.
    pub const ADDRESS: Address = u64_to_address(3);

    /// The shanghai RIPEMD160 precompile is not implemented in the Shanghai hardfork.
    ///
    /// This precompile is not implemented and will halt with "Precompile not implemented".
    pub const SHANGHAI: Precompile = precompile_not_implemented(PrecompileId::Ripemd160, ADDRESS);

    /// The maximum length of the input for the RIPEMD160 precompile in the GALDOGEOS hardfork.
    pub const GALDOGEOS_LEN_LIMIT: usize = 32;

    fn galdogeos_run(input: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
        if input.len() > GALDOGEOS_LEN_LIMIT {
            return Ok(PrecompileOutput::halt(
                PrecompileHalt::other_static("Ripemd160InputOverflow: ripemd160 input overflow"),
                reservoir,
            ));
        }
        Ok(call_eth_precompile(hash::ripemd160_run, input, gas_limit, reservoir))
    }

    /// The GALDOGEOS RIPEMD160 is enabled.
    pub const GALDOGEOS: Precompile =
        Precompile::new(PrecompileId::Ripemd160, ADDRESS, galdogeos_run);
}
