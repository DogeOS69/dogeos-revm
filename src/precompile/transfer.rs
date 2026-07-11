use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use once_cell::race::OnceBox;
use revm::precompile::{
    u64_to_address, PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult,
};
use revm_primitives::{address, Address, U256};
use std::{borrow::Cow, boxed::Box, format};

/// The Transfer precompile address.
pub const ADDRESS: Address = u64_to_address(0xff - 2);

/// The native DOGE token address (only allowed caller for the transfer precompile).
pub const NATIVE_DOGE_TOKEN_ADDRESS: Address =
    address!("0x530000000000000000000000000000000000d09e");

/// The Transfer precompile id.
pub const ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("TRANSFER"));

/// The Transfer precompile gas cost.
pub const GAS_COST: u64 = 9000;

const CALL_DATA_LENGTH: usize = 32 + 32 + 32; // 3 parameters, each 32 bytes

/// Returns the lazily initialized native transfer precompile.
pub fn precompile() -> &'static DynPrecompile {
    static INSTANCE: OnceBox<DynPrecompile> = OnceBox::new();
    INSTANCE.get_or_init(|| Box::new(DynPrecompile::new_stateful(ID, run)))
}

fn run(mut input: PrecompileInput<'_>) -> PrecompileResult {
    // 1. can't transfer during static call
    if input.is_static {
        return Err(PrecompileError::other_static(
            "transfer precompile cannot be called in a static context",
        ));
    }

    // 2. only allow DOGE token contract to call this precompile
    if !input.is_direct_call() {
        return Err(PrecompileError::other(
            "transfer precompile must be called directly, not via delegatecall or callcode",
        ));
    }
    if *input.caller() != NATIVE_DOGE_TOKEN_ADDRESS {
        return Err(PrecompileError::other(
            "transfer precompile can only be called by the allowed caller",
        ));
    }

    if input.gas < GAS_COST {
        return Err(PrecompileError::OutOfGas);
    }

    if input.data().len() != CALL_DATA_LENGTH {
        return Err(PrecompileError::other(
            "transfer precompile expects exactly 96 bytes of calldata",
        ));
    }

    let from = Address::from_slice(&input.data()[12..32]);
    let to = Address::from_slice(&input.data()[44..64]);
    let value = U256::from_be_slice(&input.data()[64..96]);

    match input.internals_mut().transfer(from, to, value) {
        Ok(None) => Ok(PrecompileOutput::new(GAS_COST, Default::default())),
        Ok(Some(_transfer_error)) => {
            Ok(PrecompileOutput::new_reverted(GAS_COST, Default::default()))
        }
        Err(db_error) => Err(PrecompileError::Fatal(format!(
            "transfer failed due to database error: {:?}",
            db_error
        ))),
    }
}
