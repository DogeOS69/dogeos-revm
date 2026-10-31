//! Addresses and IDs of the precompiles installed by the Scroll specs.
//!
//! Every precompile address used by this crate is defined here. Precompiles taken unchanged from
//! revm carry revm's own address; the tests check that it matches the constant below.

use revm::precompile::{u64_to_address, PrecompileId};
use revm_primitives::Address;
use std::borrow::Cow;

/// The ECRECOVER precompile address.
pub const ECRECOVER: Address = u64_to_address(1);

/// The SHA-256 precompile address.
pub const SHA256: Address = u64_to_address(2);

/// The RIPEMD160 precompile address.
pub const RIPEMD160: Address = u64_to_address(3);

/// The IDENTITY precompile address.
pub const IDENTITY: Address = u64_to_address(4);

/// The MODEXP precompile address.
pub const MODEXP: Address = u64_to_address(5);

/// The BN254 point addition precompile address.
pub const BN254_ADD: Address = u64_to_address(6);

/// The BN254 scalar multiplication precompile address.
pub const BN254_MUL: Address = u64_to_address(7);

/// The BN254 pairing precompile address.
pub const BN254_PAIRING: Address = u64_to_address(8);

/// The BLAKE2 precompile address.
pub const BLAKE2F: Address = u64_to_address(9);

/// The P256VERIFY precompile address.
pub const P256VERIFY: Address = u64_to_address(256);

/// The Transfer precompile address.
///
/// The native DOGE token contract also hardcodes this address.
pub const TRANSFER: Address = u64_to_address(0xff - 2);

/// The Transfer precompile id.
pub const TRANSFER_ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("TRANSFER"));
