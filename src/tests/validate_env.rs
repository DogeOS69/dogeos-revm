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
use revm_primitives::{eip7825, Address, B256};

fn assert_matches_mainnet(ctx: ScrollContext<InMemoryDB>, case: &str) {
    let mut scroll_evm = ctx.clone().build_scroll();
    let mut mainnet_evm = ctx.build_scroll();
    let scroll_handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mainnet_handler = MainnetHandler::<_, EVMError<_>, EthFrame<_>>::default();

    let scroll_result = scroll_handler.validate_env(&mut scroll_evm);
    let mainnet_result = mainnet_handler.validate_env(&mut mainnet_evm);

    assert_eq!(scroll_result, mainnet_result, "validation drift for {case}");
}

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
fn access_list_entries_are_charged_as_intrinsic_gas(
) -> Result<(), EVMError<core::convert::Infallible>> {
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
            AccessListItem {
                address: Address::from([4; 20]),
                storage_keys: vec![],
            },
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
