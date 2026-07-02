use revm::precompile::{
    bn254::{self, run_pair, PAIR_ELEMENT_LEN},
    PrecompileHalt, PrecompileOutput, PrecompileResult,
};

pub mod pair {
    use super::*;

    pub use bn254::pair::{ADDRESS, ISTANBUL_PAIR_BASE, ISTANBUL_PAIR_PER_POINT};
    use revm::precompile::{Precompile, PrecompileId};

    /// The number of pairing inputs per pairing operation. If the inputs provided to the precompile
    /// call are < 4, we append (G1::infinity, G2::generator) until we have the required no. of
    /// inputs.
    const BERNOULLI_LEN_LIMIT: usize = 4;

    /// The Bn254 pair precompile with BERNOULLI input rules.
    pub const BERNOULLI: Precompile =
        Precompile::new(PrecompileId::Bn254Pairing, ADDRESS, bernoulli_run);

    /// The bernoulli Bn254 pair precompile implementation.
    ///
    /// Inputs longer than four pairing elements halt this precompile call without aborting the
    /// transaction.
    fn bernoulli_run(input: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
        if input.len() > BERNOULLI_LEN_LIMIT * PAIR_ELEMENT_LEN {
            return Ok(PrecompileOutput::halt(
                PrecompileHalt::other_static("BN128PairingInputOverflow: input overflow"),
                reservoir,
            ));
        }
        Ok(PrecompileOutput::from_eth_result(
            run_pair(input, ISTANBUL_PAIR_PER_POINT, ISTANBUL_PAIR_BASE, gas_limit),
            reservoir,
        ))
    }

    /// The Bn254 pair precompile in FEYNMAN hardfork.
    pub const FEYNMAN: Precompile = bn254::pair::ISTANBUL;
}
