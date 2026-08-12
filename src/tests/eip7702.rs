use crate::{
    builder::{ScrollBuilder, ScrollCfgExt},
    handler::ScrollHandler,
    test_utils::{context, ScrollContextTestUtils},
    ScrollSpecId,
};
use std::{boxed::Box, vec};

use revm::{
    context::{
        either::Either,
        result::{EVMError, InvalidTransaction, ResultAndState},
        transaction::{
            Authorization, RecoveredAuthority, RecoveredAuthorization, SignedAuthorization,
        },
        TransactionType,
    },
    database::DbAccount,
    handler::{EthFrame, Handler},
    state::AccountInfo,
    ExecuteEvm,
};
use revm_primitives::{address, eip7702, Address, U256};

/// The recovered authority of the refund test's authorization.
const AUTHORITY: Address = address!("0x00000000000000000000000000000000000000aa");
/// The delegation target designated by the refund test's authorization.
const DELEGATE: Address = address!("0x00000000000000000000000000000000000000bb");

fn authorization() -> SignedAuthorization {
    SignedAuthorization::new_unchecked(
        Authorization { chain_id: Default::default(), address: Default::default(), nonce: 0 },
        0,
        U256::ZERO,
        U256::ZERO,
    )
}

/// Returns a valid recovered authorization for [`AUTHORITY`], bypassing signature recovery.
fn recovered_authorization(nonce: u64) -> Either<SignedAuthorization, RecoveredAuthorization> {
    Either::Right(RecoveredAuthorization::new_unchecked(
        Authorization { chain_id: U256::ZERO, address: DELEGATE, nonce },
        RecoveredAuthority::Valid(AUTHORITY),
    ))
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

#[test]
fn test_euclid_existing_authority_refund_reduces_final_gas(
) -> Result<(), Box<dyn core::error::Error>> {
    // 2_000 nonzero calldata bytes make the transaction spend 78_000 gas (21_000 base +
    // 32_000 calldata + 25_000 authorization), so the EIP-3529 one-fifth refund cap
    // (78_000 / 5 = 15_600) does not mask the 12_500 existing-authority refund.
    const CALLDATA_LEN: usize = 2_000;
    const SPENT_GAS: u64 = 78_000;
    const EXISTING_AUTHORITY_REFUND: u64 = 12_500;
    // Pin the historical values as literals and surface upstream constant drift here instead of
    // silently tracking a changed dependency in the assertions below.
    const {
        assert!(SPENT_GAS == 21_000 + 16 * CALLDATA_LEN as u64 + eip7702::PER_EMPTY_ACCOUNT_COST);
        assert!(
            EXISTING_AUTHORITY_REFUND ==
                eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST
        );
    }

    let run = |authority_exists: bool| -> Result<u64, Box<dyn core::error::Error>> {
        let ctx = context()
            .with_funds(U256::from(1_000_000u64))
            .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::EUCLID))
            .modify_tx_chained(|tx| {
                tx.base.tx_type = TransactionType::Eip7702 as u8;
                tx.base.gas_limit = 100_000;
                tx.base.data = vec![0xff; CALLDATA_LEN].into();
                tx.base.authorization_list = vec![recovered_authorization(0)];
            })
            .modify_db_chained(|db| {
                if authority_exists {
                    db.cache.accounts.insert(
                        AUTHORITY,
                        DbAccount {
                            info: AccountInfo { balance: U256::ONE, ..Default::default() },
                            ..Default::default()
                        },
                    );
                }
            });
        let tx = ctx.tx.clone();
        let mut evm = ctx.build_scroll();
        let ResultAndState { result, .. } = evm.transact(tx)?;
        assert!(result.is_success(), "expected success, got {result:?}");
        Ok(result.gas_used())
    };

    // An authorization whose authority account already exists refunds
    // `PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST` (12_500) gas; a nonexistent authority
    // refunds nothing. Both must reach the public final gas result.
    let existing_authority_gas = run(true)?;
    let nonexistent_authority_gas = run(false)?;

    assert_eq!(nonexistent_authority_gas, SPENT_GAS);
    assert_eq!(existing_authority_gas, SPENT_GAS - EXISTING_AUTHORITY_REFUND);
    assert_eq!(nonexistent_authority_gas - existing_authority_gas, EXISTING_AUTHORITY_REFUND);

    Ok(())
}
