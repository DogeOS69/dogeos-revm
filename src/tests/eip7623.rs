use crate::{
    builder::ScrollBuilder, handler::ScrollHandler, test_utils::context_with_spec, ScrollSpecId,
};

use revm::{
    context::result::{EVMError, InvalidTransaction},
    handler::{EthFrame, Handler},
};
use revm_primitives::{bytes, Bytes};

fn calldata_cases() -> [(Bytes, u64); 3] {
    [(bytes!("0x00"), 21_010), (bytes!("0x01"), 21_040), (bytes!("0x000102"), 21_090)]
}

fn validate_initial_tx_gas(
    spec: ScrollSpecId,
    calldata: Bytes,
    gas_limit: u64,
) -> Result<revm::interpreter::InitialAndFloorGas, EVMError<core::convert::Infallible>> {
    let mut evm = context_with_spec(spec)
        .modify_tx_chained(|tx| {
            tx.base.gas_limit = gas_limit;
            tx.base.data = calldata;
        })
        .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    handler.validate_initial_tx_gas(&mut evm)
}

#[test]
fn test_should_not_apply_eip7623_calldata_gas_for_euclid() {
    let gas = validate_initial_tx_gas(ScrollSpecId::EUCLID, bytes!("0x000102"), 21_036).unwrap();

    assert_eq!(gas.floor_gas(), 0);
}

#[test]
fn test_should_apply_eip7623_floor_for_feynman_and_galileo_calldata_variants() {
    for spec in [ScrollSpecId::FEYNMAN, ScrollSpecId::GALILEO] {
        for (calldata, gas_floor) in calldata_cases() {
            let err = validate_initial_tx_gas(spec, calldata.clone(), gas_floor - 1).unwrap_err();
            assert_eq!(
                err,
                EVMError::Transaction(InvalidTransaction::GasFloorMoreThanGasLimit {
                    gas_limit: gas_floor - 1,
                    gas_floor,
                })
            );

            let exact = validate_initial_tx_gas(spec, calldata.clone(), gas_floor).unwrap();
            assert_eq!(exact.floor_gas(), gas_floor);

            let above = validate_initial_tx_gas(spec, calldata, gas_floor + 1).unwrap();
            assert_eq!(above.floor_gas(), gas_floor);
        }
    }
}
