use crate::{
    builder::{ScrollBuilder, ScrollCfgExt},
    handler::ScrollHandler,
    test_utils::context,
    transaction::L1_MESSAGE_TYPE,
    ScrollSpecId,
};
use std::boxed::Box;

use revm::{
    context::result::{EVMError, InvalidTransaction},
    handler::{EthFrame, Handler},
};
use revm_primitives::eip7825;

fn context_with_spec_and_gas_limit(
    spec: ScrollSpecId,
    gas_limit: u64,
) -> crate::builder::ScrollContext<revm::database::InMemoryDB> {
    context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(spec))
        .modify_tx_chained(|tx| tx.base.gas_limit = gas_limit)
        .modify_block_chained(|block| block.gas_limit = gas_limit)
}

#[test]
fn tsuki_tx_above_eip7825_cap_is_rejected() {
    let gas_limit = eip7825::TX_GAS_LIMIT_CAP + 1;
    let ctx = context_with_spec_and_gas_limit(ScrollSpecId::TSUKI, gas_limit);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let err = handler.validate_env(&mut evm).unwrap_err();
    assert_eq!(
        err,
        EVMError::Transaction(InvalidTransaction::TxGasLimitGreaterThanCap {
            gas_limit,
            cap: eip7825::TX_GAS_LIMIT_CAP
        })
    );
}

#[test]
fn tsuki_tx_at_eip7825_cap_is_accepted() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context_with_spec_and_gas_limit(ScrollSpecId::TSUKI, eip7825::TX_GAS_LIMIT_CAP);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    handler.validate(&mut evm)?;

    Ok(())
}

#[test]
fn pre_tsuki_tx_above_eip7825_cap_is_accepted() -> Result<(), Box<dyn core::error::Error>> {
    let gas_limit = eip7825::TX_GAS_LIMIT_CAP + 1;
    let ctx = context_with_spec_and_gas_limit(ScrollSpecId::GALILEO, gas_limit);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    handler.validate(&mut evm)?;

    Ok(())
}

#[test]
fn tsuki_l1_message_above_eip7825_cap_is_accepted() -> Result<(), Box<dyn core::error::Error>> {
    let gas_limit = eip7825::TX_GAS_LIMIT_CAP + 1;
    let ctx = context_with_spec_and_gas_limit(ScrollSpecId::TSUKI, gas_limit)
        .modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    assert_eq!(evm.0.ctx.cfg.tx_gas_limit_cap, Some(eip7825::TX_GAS_LIMIT_CAP));
    handler.validate(&mut evm)?;

    Ok(())
}

#[test]
fn tsuki_l1_message_still_enforces_block_gas_limit() {
    let gas_limit = eip7825::TX_GAS_LIMIT_CAP + 1;
    let ctx = context_with_spec_and_gas_limit(ScrollSpecId::TSUKI, gas_limit)
        .modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE)
        .modify_block_chained(|block| block.gas_limit = gas_limit - 1);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    assert_eq!(evm.0.ctx.cfg.tx_gas_limit_cap, Some(eip7825::TX_GAS_LIMIT_CAP));
    let err = handler.validate_env(&mut evm).unwrap_err();
    assert_eq!(err, EVMError::Transaction(InvalidTransaction::CallerGasLimitMoreThanBlock));
}

#[test]
fn explicit_tx_gas_limit_cap_override_is_preserved() {
    let gas_limit = 43;
    let cap = 42;
    let ctx = context()
        .modify_cfg_chained(|cfg| {
            cfg.set_scroll_spec(ScrollSpecId::TSUKI);
            cfg.tx_gas_limit_cap = Some(cap);
        })
        .modify_tx_chained(|tx| tx.base.gas_limit = gas_limit)
        .modify_block_chained(|block| block.gas_limit = gas_limit);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    assert_eq!(evm.0.ctx.cfg.tx_gas_limit_cap, Some(cap));
    let err = handler.validate_env(&mut evm).unwrap_err();
    assert_eq!(
        err,
        EVMError::Transaction(InvalidTransaction::TxGasLimitGreaterThanCap { gas_limit, cap })
    );
}
