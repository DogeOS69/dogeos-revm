use crate::{
    builder::ScrollBuilder, handler::ScrollHandler, test_utils::context_with_spec, ScrollSpecId,
};

use revm::{
    context::{
        result::{EVMError, InvalidTransaction},
        TransactionType,
    },
    handler::{EthFrame, Handler},
};

#[test]
fn test_validate_env_rejects_eip4844_for_feynman_and_galileo() {
    for spec in [ScrollSpecId::FEYNMAN, ScrollSpecId::GALILEO] {
        let mut evm = context_with_spec(spec)
            .modify_tx_chained(|tx| {
                tx.base.tx_type = TransactionType::Eip4844 as u8;
                tx.base.chain_id = Some(1);
                tx.base.gas_limit = 100_000;
            })
            .build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

        let err = handler.validate_env(&mut evm).unwrap_err();
        assert_eq!(err, EVMError::Transaction(InvalidTransaction::Eip4844NotSupported));
    }
}
