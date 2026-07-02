use crate::{builder::ScrollBuilder, handler::ScrollHandler, test_utils::context};

use revm::{
    context::{result::InvalidTransaction, transaction::TransactionType},
    context_interface::result::EVMError,
    handler::{EthFrame, Handler},
};
use revm_primitives::{bytes, TxKind};

fn validate_env_err(
    ctx: crate::builder::ScrollContext<revm::database::InMemoryDB>,
) -> InvalidTransaction {
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    match handler.validate_env(&mut evm).unwrap_err() {
        EVMError::Transaction(err) => err,
        err => panic!("expected transaction validation error, got {err:?}"),
    }
}

#[test]
fn test_validate_env_rejects_invalid_chain_id() {
    let err = validate_env_err(context().modify_tx_chained(|tx| tx.base.chain_id = Some(2)));

    assert_eq!(err, InvalidTransaction::InvalidChainId);
}

#[test]
fn test_validate_env_rejects_missing_chain_id_for_typed_transaction() {
    let err = validate_env_err(context().modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip1559 as u8;
        tx.base.chain_id = None;
        tx.base.gas_priority_fee = Some(0);
    }));

    assert_eq!(err, InvalidTransaction::MissingChainId);
}

#[test]
fn test_validate_env_rejects_tx_gas_limit_above_cap() {
    let err = validate_env_err(
        context()
            .modify_cfg_chained(|cfg| cfg.tx_gas_limit_cap = Some(29_999))
            .modify_tx_chained(|tx| tx.base.gas_limit = 30_000),
    );

    assert_eq!(
        err,
        InvalidTransaction::TxGasLimitGreaterThanCap { gas_limit: 30_000, cap: 29_999 }
    );
}

#[test]
fn test_validate_env_rejects_tx_gas_limit_above_block_limit() {
    let err = validate_env_err(
        context()
            .modify_block_chained(|block| block.gas_limit = 29_999)
            .modify_tx_chained(|tx| tx.base.gas_limit = 30_000),
    );

    assert_eq!(err, InvalidTransaction::CallerGasLimitMoreThanBlock);
}

#[test]
fn test_validate_env_rejects_create_initcode_size_limit() {
    let err = validate_env_err(
        context()
            .modify_cfg_chained(|cfg| cfg.limit_contract_initcode_size = Some(2))
            .modify_tx_chained(|tx| {
                tx.base.kind = TxKind::Create;
                tx.base.data = bytes!("0x010203");
            }),
    );

    assert_eq!(err, InvalidTransaction::CreateInitCodeSizeLimit);
}

#[test]
fn test_validate_env_rejects_nonce_overflow() {
    let err = validate_env_err(context().modify_tx_chained(|tx| tx.base.nonce = u64::MAX));

    assert_eq!(err, InvalidTransaction::NonceOverflowInTransaction);
}
