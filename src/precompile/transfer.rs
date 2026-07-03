use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use revm::precompile::{
    u64_to_address, PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult,
};
use revm_primitives::{Address, U256};
use std::{borrow::Cow, format};

/// The Transfer precompile address.
pub const ADDRESS: Address = u64_to_address(0xff - 2);

/// The Transfer precompile id.
pub const ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("TRANSFER"));

/// The Transfer precompile gas cost.
pub const GAS_COST: u64 = 9000;

const CALL_DATA_LENGTH: usize = 32 + 32 + 32; // 3 parameters, each 32 bytes

pub fn tsuki(allow_transfer_caller: Address) -> DynPrecompile {
    DynPrecompile::new_stateful(ID, move |input| run(input, allow_transfer_caller))
}

fn run(mut input: PrecompileInput<'_>, allow_transfer_caller: Address) -> PrecompileResult {
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
    if *input.caller() != allow_transfer_caller {
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
