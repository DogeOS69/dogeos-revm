use crate::{
    builder::{ScrollBuilder, ScrollCfgExt, ScrollContext},
    handler::ScrollHandler,
    test_utils::context,
    ScrollSpecId,
};
use revm::{
    context::{
        result::{EVMError, InvalidTransaction},
        transaction::{AccessList, AccessListItem},
        TransactionType,
    },
    context_interface::cfg::gas,
    database::InMemoryDB,
    handler::{EthFrame, Handler, MainnetHandler},
};
use revm_primitives::{eip3860, eip7825, Address, TxKind, B256};
use std::vec;

type ValidationResult = Result<(), EVMError<core::convert::Infallible>>;

/// Runs Scroll's `validate_env` and the mapped mainnet `validate_env` on clones of the same
/// context. The mainnet handler intentionally receives a Scroll context so both handlers see
/// identical transaction, block, and configuration inputs.
fn validate_scroll_and_mainnet(
    ctx: ScrollContext<InMemoryDB>,
) -> (ValidationResult, ValidationResult) {
    let mut scroll_evm = ctx.clone().build_scroll();
    let mut mainnet_evm = ctx.build_scroll();
    let scroll_handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mainnet_handler = MainnetHandler::<_, EVMError<_>, EthFrame<_>>::default();

    let scroll_result = scroll_handler.validate_env(&mut scroll_evm);
    let mainnet_result = mainnet_handler.validate_env(&mut mainnet_evm);

    (scroll_result, mainnet_result)
}

fn assert_matches_mainnet(ctx: ScrollContext<InMemoryDB>, case: &str) {
    let (scroll_result, mainnet_result) = validate_scroll_and_mainnet(ctx);
    assert_eq!(scroll_result, mainnet_result, "validation drift for {case}");
}

/// Asserts both handlers reject with `expected`, guarding against a vacuous parity check where
/// both handlers accept because the case never reached the target rejection branch.
fn assert_rejects_like_mainnet(
    ctx: ScrollContext<InMemoryDB>,
    case: &str,
    expected: InvalidTransaction,
) {
    let (scroll_result, mainnet_result) = validate_scroll_and_mainnet(ctx);
    assert_eq!(
        scroll_result,
        Err(EVMError::Transaction(expected)),
        "unexpected scroll result for {case}"
    );
    assert_eq!(scroll_result, mainnet_result, "validation drift for {case}");
}

// NOTE: the `disable_base_fee`, `disable_priority_fee_check`, and `disable_block_gas_limit`
// switches are behind revm's `optional_no_base_fee`, `optional_priority_fee_check`, and
// `optional_block_gas_limit` features, which this crate does not enable. In this build the
// corresponding checks are always enabled, so only the enabled-path rejections are testable.
// The chain-id check toggle is a plain field and is covered by
// `chain_id_validation_matches_ethereum_rules`.

#[test]
fn non_l1_validation_matches_mainnet_across_transaction_types() {
    let cases = [
        ("legacy", TransactionType::Legacy as u8),
        ("eip2930", TransactionType::Eip2930 as u8),
        ("eip1559", TransactionType::Eip1559 as u8),
        ("eip4844", TransactionType::Eip4844 as u8),
        ("eip7702-before-euclid", TransactionType::Eip7702 as u8),
        ("custom", 0x7f),
    ];

    for (case, tx_type) in cases {
        let ctx = context()
            .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
            .modify_tx_chained(|tx| {
                tx.base.tx_type = tx_type;
                tx.base.chain_id = Some(1);
            });

        assert_matches_mainnet(ctx, case);
    }
}

#[test]
fn non_l1_tsuki_gas_cap_matches_mainnet() {
    let gas_limit = eip7825::TX_GAS_LIMIT_CAP + 1;
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::TSUKI))
        .modify_tx_chained(|tx| tx.base.gas_limit = gas_limit)
        .modify_block_chained(|block| block.gas_limit = gas_limit);

    assert_matches_mainnet(ctx, "tsuki transaction gas limit cap");
}

#[test]
fn chain_id_validation_matches_ethereum_rules() {
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let ctx = context().modify_tx_chained(|tx| tx.base.chain_id = Some(2));
    let mut evm = ctx.build_scroll();
    assert_eq!(
        handler.validate_env(&mut evm),
        Err(EVMError::Transaction(InvalidTransaction::InvalidChainId))
    );

    let ctx = context().modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip1559 as u8;
        tx.base.chain_id = None;
    });
    let mut evm = ctx.build_scroll();
    assert_eq!(
        handler.validate_env(&mut evm),
        Err(EVMError::Transaction(InvalidTransaction::MissingChainId))
    );

    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.tx_chain_id_check = false)
        .modify_tx_chained(|tx| tx.base.chain_id = Some(2));
    let mut evm = ctx.build_scroll();
    assert!(handler.validate_env(&mut evm).is_ok());
}

#[test]
fn access_list_entries_are_charged_as_intrinsic_gas() -> ValidationResult {
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let ctx = context().modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip2930 as u8;
        tx.base.gas_limit = 30_000;
    });
    let mut evm = ctx.build_scroll();
    let without_access_list = handler.validate_initial_tx_gas(&mut evm)?;

    let ctx = context().modify_tx_chained(|tx| {
        tx.base.tx_type = TransactionType::Eip2930 as u8;
        tx.base.gas_limit = 30_000;
        tx.base.access_list = AccessList(vec![
            AccessListItem {
                address: Address::from([1; 20]),
                storage_keys: vec![B256::from([2; 32]), B256::from([3; 32])],
            },
            AccessListItem { address: Address::from([4; 20]), storage_keys: vec![] },
        ]);
    });
    let mut evm = ctx.build_scroll();
    let with_access_list = handler.validate_initial_tx_gas(&mut evm)?;

    assert_eq!(
        with_access_list.initial_gas - without_access_list.initial_gas,
        2 * gas::ACCESS_LIST_ADDRESS + 2 * gas::ACCESS_LIST_STORAGE_KEY
    );

    Ok(())
}

#[test]
fn legacy_and_eip2930_gas_price_below_base_fee_match_mainnet() {
    for (case, tx_type) in
        [("legacy", TransactionType::Legacy), ("eip2930", TransactionType::Eip2930)]
    {
        let ctx = context()
            .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
            .modify_block_chained(|block| block.basefee = 100)
            .modify_tx_chained(|tx| {
                tx.base.tx_type = tx_type as u8;
                tx.base.chain_id = Some(1);
                tx.base.gas_price = 99;
            });

        assert_rejects_like_mainnet(ctx, case, InvalidTransaction::GasPriceLessThanBasefee);
    }
}

#[test]
fn eip1559_priority_fee_above_max_fee_matches_mainnet() {
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
        .modify_tx_chained(|tx| {
            tx.base.tx_type = TransactionType::Eip1559 as u8;
            tx.base.chain_id = Some(1);
            tx.base.gas_price = 10;
            tx.base.gas_priority_fee = Some(11);
        });

    assert_rejects_like_mainnet(
        ctx,
        "eip1559 priority fee above max fee",
        InvalidTransaction::PriorityFeeGreaterThanMaxFee,
    );
}

#[test]
fn tx_gas_limit_above_block_gas_limit_matches_mainnet() {
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
        .modify_block_chained(|block| block.gas_limit = 21_000)
        .modify_tx_chained(|tx| tx.base.gas_limit = 21_001);

    assert_rejects_like_mainnet(
        ctx,
        "transaction gas limit above block gas limit",
        InvalidTransaction::CallerGasLimitMoreThanBlock,
    );
}

#[test]
fn create_initcode_above_size_limit_matches_mainnet() {
    let ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
        .modify_tx_chained(|tx| {
            tx.base.kind = TxKind::Create;
            tx.base.data = vec![0u8; eip3860::MAX_INITCODE_SIZE + 1].into();
        });

    assert_rejects_like_mainnet(
        ctx,
        "create initcode above size limit",
        InvalidTransaction::CreateInitCodeSizeLimit,
    );
}

#[test]
fn legacy_transaction_access_list_is_ignored_for_intrinsic_gas() -> ValidationResult {
    // A valid encoded legacy transaction cannot carry an access list; this synthetic state pins
    // the explicit legacy guard in `validate_initial_tx_gas`, matching the mapped mainnet rules.
    let scroll_handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mainnet_handler = MainnetHandler::<_, EVMError<_>, EthFrame<_>>::default();

    let base_ctx = context()
        .modify_cfg_chained(|cfg| cfg.set_scroll_spec(ScrollSpecId::DARWIN))
        .modify_tx_chained(|tx| tx.base.gas_limit = 30_000);
    let with_access_list_ctx = base_ctx.clone().modify_tx_chained(|tx| {
        tx.base.access_list = AccessList(vec![AccessListItem {
            address: Address::from([1; 20]),
            storage_keys: vec![B256::from([2; 32])],
        }]);
    });

    let mut evm = base_ctx.build_scroll();
    let without_access_list = scroll_handler.validate_initial_tx_gas(&mut evm)?;

    let mut scroll_evm = with_access_list_ctx.clone().build_scroll();
    let with_access_list = scroll_handler.validate_initial_tx_gas(&mut scroll_evm)?;

    let mut mainnet_evm = with_access_list_ctx.build_scroll();
    let mainnet_with_access_list = mainnet_handler.validate_initial_tx_gas(&mut mainnet_evm)?;

    assert_eq!(with_access_list, without_access_list);
    assert_eq!(with_access_list, mainnet_with_access_list);

    Ok(())
}
