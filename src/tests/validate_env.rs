use crate::{
    builder::{ScrollBuilder, ScrollCfgExt, ScrollContext},
    handler::ScrollHandler,
    test_utils::context,
    ScrollSpecId,
};
use revm::{
    context::{result::EVMError, TransactionType},
    database::InMemoryDB,
    handler::{EthFrame, Handler, MainnetHandler},
};
use revm_primitives::eip7825;

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
