use super::{addresses, precompile_not_implemented};

use revm::precompile::{Precompile, PrecompileId};

/// The BLAKE2 precompile is not implemented in the SHANGHAI hardfork.
///
/// This precompile is not implemented and will return `PrecompileError::Other("Precompile not
/// implemented".into())`.
pub const SHANGHAI: Precompile =
    precompile_not_implemented(PrecompileId::Blake2F, addresses::BLAKE2F);
