use crate::{
    builder::ScrollBuilder,
    handler::ScrollHandler,
    test_utils::{context_with_spec, feynman_context},
    ScrollSpecId,
};
use std::{boxed::Box, vec::Vec};

use revm::{
    context::{
        either::Either,
        result::{EVMError, InvalidTransaction},
        transaction::{
            Authorization, RecoveredAuthority, RecoveredAuthorization, SignedAuthorization,
        },
        ContextTr, JournalTr, TransactionType,
    },
    database::DbAccount,
    handler::{EthFrame, EvmTr, Handler},
    interpreter::InitialAndFloorGas,
    state::AccountInfo,
};
use revm_primitives::{address, eip7702, Address, U256};

type TestAuthorization = Either<SignedAuthorization, RecoveredAuthorization>;
type TestResult = Result<(), Box<dyn core::error::Error>>;

const AUTHORITY: Address = address!("0x000000000000000000000000000000000000a770");
const DELEGATE: Address = address!("0x000000000000000000000000000000000000d770");

fn invalid_authorization() -> TestAuthorization {
    Either::Left(SignedAuthorization::new_unchecked(
        Authorization {
            chain_id: U256::ZERO,
            address: address!("0x0000000000000000000000000000000000007702"),
            nonce: 0,
        },
        0,
        U256::ZERO,
        U256::ZERO,
    ))
}

fn recovered_authorization(chain_id: U256, address: Address, nonce: u64) -> TestAuthorization {
    Either::Right(RecoveredAuthorization::new_unchecked(
        Authorization { chain_id, address, nonce },
        RecoveredAuthority::Valid(AUTHORITY),
    ))
}

fn eip7702_context(
    spec: ScrollSpecId,
    auth_list_len: usize,
) -> crate::builder::ScrollContext<revm::database::InMemoryDB> {
    eip7702_context_with_authorizations(
        spec,
        (0..auth_list_len).map(|_| invalid_authorization()).collect(),
    )
}

fn eip7702_context_with_authorizations(
    spec: ScrollSpecId,
    authorization_list: Vec<TestAuthorization>,
) -> crate::builder::ScrollContext<revm::database::InMemoryDB> {
    context_with_spec(spec).modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip7702 as u8;
        tx.base.chain_id = Some(1);
        tx.base.gas_limit = 100_000;
        tx.base.authorization_list = authorization_list;
    })
}

#[test]
fn test_validate_initial_gas_eip7702_charges_authorization_list() -> TestResult {
    let mut evm =
        feynman_context().modify_tx_chained(|tx| tx.base.gas_limit = 100_000).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let gas_without_authorization_list = handler.validate_initial_tx_gas(&mut evm)?;

    let mut evm = eip7702_context(ScrollSpecId::FEYNMAN, 1).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let gas_with_authorization_list = handler.validate_initial_tx_gas(&mut evm)?;

    assert_eq!(
        gas_without_authorization_list.initial_regular_gas() + eip7702::PER_EMPTY_ACCOUNT_COST,
        gas_with_authorization_list.initial_regular_gas()
    );

    Ok(())
}

#[test]
fn test_validate_env_accepts_eip7702_for_feynman_and_galileo() -> TestResult {
    for spec in [ScrollSpecId::FEYNMAN, ScrollSpecId::GALILEO] {
        let mut evm = eip7702_context(spec, 1).build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

        handler.validate_env(&mut evm)?;
    }

    Ok(())
}

#[test]
fn test_validate_env_rejects_empty_eip7702_authorization_list() {
    let mut evm = eip7702_context(ScrollSpecId::FEYNMAN, 0).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let err = handler.validate_env(&mut evm).unwrap_err();
    assert_eq!(err, EVMError::Transaction(InvalidTransaction::EmptyAuthorizationList));
}

#[test]
fn test_invalid_eip7702_authorization_is_ignored_during_application() -> TestResult {
    let mut evm = eip7702_context(ScrollSpecId::FEYNMAN, 1).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = InitialAndFloorGas::new(0, 0);

    let refund = handler.apply_eip7702_auth_list(&mut evm, &mut init_and_floor_gas)?;

    assert_eq!(refund, 0);
    assert_eq!(init_and_floor_gas.state_refund, 0);

    Ok(())
}

#[test]
fn test_valid_eip7702_authorization_delegates_empty_account() -> TestResult {
    let mut evm = eip7702_context_with_authorizations(
        ScrollSpecId::FEYNMAN,
        vec![recovered_authorization(U256::from(1), DELEGATE, 0)],
    )
    .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = InitialAndFloorGas::new(0, 0);

    let refund = handler.apply_eip7702_auth_list(&mut evm, &mut init_and_floor_gas)?;

    assert_eq!(refund, 0);
    assert_eq!(init_and_floor_gas.state_refund, 0);

    let authority = evm.ctx_mut().journal_mut().load_account(AUTHORITY)?;
    assert_eq!(authority.data.info.nonce, 1);
    let code = authority.data.info.code.as_ref().expect("authority has delegated code");
    assert_eq!(code.eip7702_address(), Some(DELEGATE));

    Ok(())
}

#[test]
fn test_valid_eip7702_authorization_refunds_existing_account() -> TestResult {
    let mut evm = eip7702_context_with_authorizations(
        ScrollSpecId::FEYNMAN,
        vec![recovered_authorization(U256::from(1), DELEGATE, 0)],
    )
    .modify_db_chained(|db| {
        db.cache.accounts.insert(
            AUTHORITY,
            DbAccount {
                info: AccountInfo { balance: U256::ONE, ..Default::default() },
                ..Default::default()
            },
        );
    })
    .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = InitialAndFloorGas::new(0, 0);

    let refund = handler.apply_eip7702_auth_list(&mut evm, &mut init_and_floor_gas)?;

    assert_eq!(refund, eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST);

    Ok(())
}

#[test]
fn test_wrong_chain_eip7702_authorization_is_ignored_during_application() -> TestResult {
    let mut evm = eip7702_context_with_authorizations(
        ScrollSpecId::FEYNMAN,
        vec![recovered_authorization(U256::from(2), DELEGATE, 0)],
    )
    .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = InitialAndFloorGas::new(0, 0);

    let refund = handler.apply_eip7702_auth_list(&mut evm, &mut init_and_floor_gas)?;

    assert_eq!(refund, 0);
    assert_eq!(init_and_floor_gas.state_refund, 0);
    let authority = evm.ctx_mut().journal_mut().load_account(AUTHORITY)?;
    assert_eq!(authority.data.info.nonce, 0);
    if let Some(code) = authority.data.info.code.as_ref() {
        assert_eq!(code.eip7702_address(), None);
    }

    Ok(())
}

#[test]
fn test_nonce_mismatch_eip7702_authorization_is_ignored_during_application() -> TestResult {
    let mut evm = eip7702_context_with_authorizations(
        ScrollSpecId::FEYNMAN,
        vec![recovered_authorization(U256::from(1), DELEGATE, 0)],
    )
    .modify_db_chained(|db| {
        db.cache.accounts.insert(
            AUTHORITY,
            DbAccount {
                info: AccountInfo { nonce: 7, balance: U256::ONE, ..Default::default() },
                ..Default::default()
            },
        );
    })
    .build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = InitialAndFloorGas::new(0, 0);

    let refund = handler.apply_eip7702_auth_list(&mut evm, &mut init_and_floor_gas)?;

    assert_eq!(refund, 0);
    assert_eq!(init_and_floor_gas.state_refund, 0);
    let authority = evm.ctx_mut().journal_mut().load_account(AUTHORITY)?;
    assert_eq!(authority.data.info.nonce, 7);
    if let Some(code) = authority.data.info.code.as_ref() {
        assert_eq!(code.eip7702_address(), None);
    }

    Ok(())
}
