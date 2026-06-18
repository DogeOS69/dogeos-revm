use super::precompile_not_implemented;

use revm::{
    precompile::{u64_to_address, Precompile, PrecompileId},
    primitives::Address,
};

/// The BLAKE2 precompile address.
pub const ADDRESS: Address = u64_to_address(9);

/// The BLAKE2 precompile is not implemented in the SHANGHAI hardfork.
///
/// This precompile is not implemented and will halt with "Precompile not implemented".
pub const SHANGHAI: Precompile = precompile_not_implemented(PrecompileId::Blake2F, ADDRESS);
