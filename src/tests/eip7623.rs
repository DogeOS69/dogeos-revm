use crate::{
    builder::{ScrollBuilder, ScrollCfgExt},
    handler::ScrollHandler,
    test_utils::{
        context, long_calldata, ScrollContextTestUtils, LONG_CALLDATA_FLOOR_GAS,
        LONG_CALLDATA_INTRINSIC_GAS,
    },
    ScrollSpecId,
};
use std::boxed::Box;

use revm::{
    context::result::{EVMError, InvalidTransaction, ResultAndState},
    handler::{EthFrame, Handler},
    ExecuteEvm,
};
use revm_primitives::{bytes, U256};

#[test]
fn test_should_not_apply_eip7623_calldata_gas_for_euclid() {
    const GAS_LIMIT: u64 = 21_032;

    // initiate handler. The `0xdead` calldata makes the intrinsic gas match the gas limit
    // exactly, while the Feynman twin test shows the same setup fails on the EIP-7623 floor.
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::EUCLID))
        .modify_tx_chained(|tx| {
            tx.base.gas_limit = GAS_LIMIT;
            tx.base.data = bytes!("0xdead");
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    // check call passes.
    let _ = handler.validate_initial_tx_gas(&mut evm).unwrap();
}

#[test]
fn test_should_charge_floor_gas_for_feynman_transaction() -> Result<(), Box<dyn core::error::Error>>
{
    const GAS_LIMIT: u64 = 30_000;

    let ctx = context()
        .with_funds(U256::from(10_000_000u64))
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::FEYNMAN))
        .modify_tx_chained(|tx| {
            tx.base.gas_limit = GAS_LIMIT;
            tx.base.data = long_calldata();
            // Feynman L1 fee computation requires a compression ratio (scaled by 1e9).
            tx.compression_ratio = Some(U256::from(5_000_000_000u64));
        });
    let tx = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let initial_gas = handler.validate_initial_tx_gas(&mut evm)?;
    assert_eq!(initial_gas.initial_gas, LONG_CALLDATA_INTRINSIC_GAS);
    assert_eq!(initial_gas.floor_gas, LONG_CALLDATA_FLOOR_GAS);

    // An ordinary (non-L1-message) transaction must charge the EIP-7623 floor in the final
    // public execution result, not only during validation.
    let ResultAndState { result, .. } = evm.transact(tx)?;
    assert!(result.is_success(), "expected success, got {result:?}");
    assert_eq!(result.gas_used(), LONG_CALLDATA_FLOOR_GAS);

    Ok(())
}

#[test]
fn test_should_apply_eip7623_calldata_gas_for_feynman() {
    const GAS_LIMIT: u64 = 21_032;
    const GAS_FLOOR: u64 = 21_080;

    // initiate handler.
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::FEYNMAN))
        .modify_tx_chained(|tx| {
            tx.base.gas_limit = GAS_LIMIT;
            tx.base.data = bytes!("0xdead");
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    // check call errors on gas floor more than gas limit.
    let err = handler.validate_initial_tx_gas(&mut evm).unwrap_err();
    assert_eq!(
        err,
        EVMError::Transaction(InvalidTransaction::GasFloorMoreThanGasLimit {
            gas_limit: GAS_LIMIT,
            gas_floor: GAS_FLOOR
        })
    )
}
