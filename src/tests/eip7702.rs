use crate::{
    builder::{ScrollBuilder, ScrollCfgExt},
    handler::ScrollHandler,
    test_utils::context,
    ScrollSpecId,
};
use std::{boxed::Box, vec};

use revm::{
    context::{
        either::Either,
        result::{EVMError, InvalidTransaction},
        transaction::{Authorization, SignedAuthorization},
        TransactionType,
    },
    handler::{EthFrame, Handler},
};
use revm_primitives::{eip7702, U256};

fn authorization() -> SignedAuthorization {
    SignedAuthorization::new_unchecked(
        Authorization { chain_id: Default::default(), address: Default::default(), nonce: 0 },
        0,
        U256::ZERO,
        U256::ZERO,
    )
}

#[test]
fn test_validate_initial_gas_eip7702() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context();
    let mut evm = ctx.clone().build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let gas_empty_authorization_list = handler.validate_initial_tx_gas(&mut evm)?;

    let mut evm = ctx
        .modify_tx_chained(|tx| {
            tx.base.gas_limit += eip7702::PER_EMPTY_ACCOUNT_COST;
            tx.base.authorization_list = vec![Either::Left(authorization())]
        })
        .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let gas_with_authorization_list = handler.validate_initial_tx_gas(&mut evm)?;

    // initial gas should include eip7702 cost of authorized accounts.
    assert_eq!(
        gas_empty_authorization_list.initial_gas + eip7702::PER_EMPTY_ACCOUNT_COST,
        gas_with_authorization_list.initial_gas
    );

    Ok(())
}

#[test]
fn test_validate_env_eip7702() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip7702 as u8;
        tx.base.authorization_list = vec![Either::Left(authorization())]
    });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    // eip 7702 env checks should pass.
    handler.validate_env(&mut evm)?;

    Ok(())
}

#[test]
fn eip7702_is_rejected_before_euclid_and_requires_an_authorization() {
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
        .modify_tx_chained(|tx| {
            tx.base.tx_type = TransactionType::Eip7702 as u8;
            tx.base.authorization_list = vec![Either::Left(authorization())];
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    assert_eq!(
        handler.validate_env(&mut evm),
        Err(EVMError::Transaction(InvalidTransaction::Eip7702NotSupported))
    );

    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::EUCLID))
        .modify_tx_chained(|tx| tx.base.tx_type = TransactionType::Eip7702 as u8);
    let mut evm = ctx.build_scroll();

    assert_eq!(
        handler.validate_env(&mut evm),
        Err(EVMError::Transaction(InvalidTransaction::EmptyAuthorizationList))
    );
}
