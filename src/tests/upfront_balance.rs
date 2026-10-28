//! The combined gas + value + L1 fee upfront balance check activates at TsukiV2. Earlier specs
//! validate and deduct the mainnet cost first, then check the L1 fee against the remaining
//! balance.
use crate::{
    builder::{ScrollBuilder, ScrollContext},
    handler::ScrollHandler,
    l1block::L1BlockInfo,
    test_utils::{context_with_spec, ScrollContextTestUtils, CALLER, L1_DATA_COST, TO},
    transaction::ScrollTxTr,
    ScrollSpecId,
};
use std::{boxed::Box, string::ToString};

use revm::{
    bytecode::{opcode::STOP, Bytecode},
    context::{
        result::{EVMError, ExecutionResult, InvalidTransaction},
        ContextTr, JournalTr,
    },
    database::InMemoryDB,
    handler::{EthFrame, EvmTr, Handler},
    state::AccountInfo,
    ExecuteEvm,
};
use revm_primitives::{Bytes, TxKind, U256};
use rstest::rstest;

const GAS_LIMIT: u64 = 21_000;
const BASE_FEE: u64 = 1;

/// An EIP-1559 transfer paying `BASE_FEE` per gas with `max_fee` as its fee cap.
fn transfer(
    spec: ScrollSpecId,
    balance: U256,
    max_fee: u128,
    value: U256,
) -> ScrollContext<InMemoryDB> {
    tx(spec, TxKind::Call(TO), GAS_LIMIT, balance, max_fee, value)
}

/// An EIP-1559 transaction paying `BASE_FEE` per gas with `max_fee` as its fee cap.
fn tx(
    spec: ScrollSpecId,
    kind: TxKind,
    gas_limit: u64,
    balance: U256,
    max_fee: u128,
    value: U256,
) -> ScrollContext<InMemoryDB> {
    context_with_spec(spec)
        .with_funds(balance)
        .modify_block_chained(|block| block.basefee = BASE_FEE)
        .modify_tx_chained(|tx| {
            tx.base.tx_type = 2;
            tx.base.kind = kind;
            tx.base.gas_limit = gas_limit;
            tx.base.gas_price = max_fee;
            tx.base.gas_priority_fee = Some(0);
            tx.base.value = value;
        })
}

fn gas_cost(fee_per_gas: u64) -> U256 {
    U256::from(GAS_LIMIT) * U256::from(fee_per_gas)
}

#[rstest]
#[case(ScrollSpecId::GALILEO)]
#[case(ScrollSpecId::TSUKI)]
#[case(ScrollSpecId::TSUKI_V2)]
fn fixture_l1_fee_is_l1_data_cost(
    #[case] spec: ScrollSpecId,
) -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context_with_spec(spec);
    let info = L1BlockInfo::try_fetch(&mut ctx.journaled_state.database, spec)?;
    let fee = info.calculate_tx_l1_cost(
        ctx.tx.rlp_bytes().unwrap(),
        spec,
        ctx.tx.compression_ratio(),
        ctx.tx.compressed_size(),
    );
    assert_eq!(fee, L1_DATA_COST);
    Ok(())
}

/// `gas·effPrice + l1Fee <= balance < gas·maxFee + value + l1Fee`, with the mainnet check
/// `balance >= gas·maxFee + value` passing. Tsuki accepts the transaction and it succeeds; TsukiV2
/// rejects it before any deduction.
#[test]
fn boundary_balance_passes_at_tsuki_and_is_rejected_at_tsuki_v2(
) -> Result<(), Box<dyn core::error::Error>> {
    let max_fee = 10;
    let value = U256::from(100);
    let balance = gas_cost(max_fee) + value;
    assert!(gas_cost(BASE_FEE) + L1_DATA_COST <= balance);
    assert!(balance < gas_cost(max_fee) + value + L1_DATA_COST);

    let ctx = transfer(ScrollSpecId::TSUKI, balance, max_fee.into(), value);
    let tx = ctx.tx.clone();
    let result = ctx.build_scroll().transact(tx)?;
    assert!(
        matches!(result.result, ExecutionResult::Success { gas, .. } if gas.used() == GAS_LIMIT)
    );
    let caller = &result.state[&CALLER].info;
    assert_eq!(caller.balance, balance - gas_cost(BASE_FEE) - L1_DATA_COST - value);
    assert_eq!(caller.nonce, 1);
    assert_eq!(result.state[&TO].info.balance, value);

    let ctx = transfer(ScrollSpecId::TSUKI_V2, balance, max_fee.into(), value);
    let tx = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    assert_eq!(
        evm.transact(tx).unwrap_err(),
        EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(gas_cost(max_fee) + value + L1_DATA_COST),
            balance: Box::new(balance),
        })
    );
    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    assert_eq!(caller.info.balance, balance);
    assert_eq!(caller.info.nonce, 0);

    Ok(())
}

/// The mainnet check passes and the remaining balance covers the L1 fee but not the value. Tsuki
/// includes the transaction, charges gas and the L1 fee, and fails the value transfer; TsukiV2
/// rejects it upfront.
#[test]
fn value_shortfall_fails_in_execution_at_tsuki_and_is_rejected_at_tsuki_v2(
) -> Result<(), Box<dyn core::error::Error>> {
    let max_fee = 2;
    let value = U256::from(100_000);
    let balance = gas_cost(max_fee) + value;
    let remaining = balance - gas_cost(BASE_FEE) - L1_DATA_COST;
    assert!(remaining < value);

    let ctx = transfer(ScrollSpecId::TSUKI, balance, max_fee.into(), value);
    let tx = ctx.tx.clone();
    let result = ctx.build_scroll().transact(tx)?;
    assert!(!result.result.is_success());
    assert_eq!(result.result.gas_used(), GAS_LIMIT);
    let caller = &result.state[&CALLER].info;
    assert_eq!(caller.balance, remaining);
    assert_eq!(caller.nonce, 1);
    assert_eq!(result.state.get(&TO).map(|to| to.info.balance).unwrap_or_default(), U256::ZERO);

    let ctx = transfer(ScrollSpecId::TSUKI_V2, balance, max_fee.into(), value);
    let tx = ctx.tx.clone();
    assert_eq!(
        ctx.build_scroll().transact(tx).unwrap_err(),
        EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(balance + L1_DATA_COST),
            balance: Box::new(balance),
        })
    );

    Ok(())
}

/// The mainnet check passes but the remaining balance is one short of the L1 fee. Before
/// TsukiV2 the error reports the L1 fee against the balance left after the mainnet deduction;
/// from TsukiV2 it reports the total against the original balance.
#[rstest]
fn l1_fee_shortfall(
    #[values(ScrollSpecId::GALILEO, ScrollSpecId::TSUKI, ScrollSpecId::TSUKI_V2)]
    spec: ScrollSpecId,
    #[values(TxKind::Call(TO), TxKind::Create)] kind: TxKind,
) -> Result<(), Box<dyn core::error::Error>> {
    let gas_limit = if kind.is_call() { GAS_LIMIT } else { 53_000 };
    let gas = U256::from(gas_limit) * U256::from(BASE_FEE);
    let balance = gas + L1_DATA_COST - U256::ONE;
    let legacy = !spec.is_enabled_in(ScrollSpecId::TSUKI_V2);
    let expected = EVMError::Transaction(if legacy {
        InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(L1_DATA_COST),
            balance: Box::new(L1_DATA_COST - U256::ONE),
        }
    } else {
        InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(gas + L1_DATA_COST),
            balance: Box::new(balance),
        }
    });

    // Before TsukiV2 the handler leaves the mainnet deduction (and a call's nonce bump) in the
    // journal when the L1 fee check fails.
    let mut evm = tx(spec, kind, gas_limit, balance, BASE_FEE.into(), U256::ZERO).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    assert_eq!(handler.pre_execution(&mut evm), Err(expected.clone()));
    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    if legacy {
        assert_eq!(caller.info.balance, balance - gas);
        assert_eq!(caller.info.nonce, u64::from(kind.is_call()));
    } else {
        assert_eq!(caller.info.balance, balance);
        assert_eq!(caller.info.nonce, 0);
    }

    // Transaction execution discards the partial deduction.
    let ctx = tx(spec, kind, gas_limit, balance, BASE_FEE.into(), U256::ZERO);
    let transaction = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    assert_eq!(evm.transact(transaction).unwrap_err(), expected);
    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    assert_eq!(caller.info.balance, balance);
    assert_eq!(caller.info.nonce, 0);

    Ok(())
}

/// A balance below `gas·maxFee + value` fails the mainnet check first before TsukiV2, which
/// reports the mainnet cost without the L1 fee.
#[rstest]
#[case::galileo(ScrollSpecId::GALILEO, gas_cost(2) + U256::from(100))]
#[case::tsuki(ScrollSpecId::TSUKI, gas_cost(2) + U256::from(100))]
#[case::tsuki_v2(ScrollSpecId::TSUKI_V2, gas_cost(2) + U256::from(100) + L1_DATA_COST)]
fn mainnet_shortfall(
    #[case] spec: ScrollSpecId,
    #[case] fee: U256,
) -> Result<(), Box<dyn core::error::Error>> {
    let value = U256::from(100);
    let balance = gas_cost(2) + value - U256::ONE;
    let ctx = transfer(spec, balance, 2, value);
    let tx = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    assert_eq!(
        evm.transact(tx).unwrap_err(),
        EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(fee),
            balance: Box::new(balance),
        })
    );
    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    assert_eq!(caller.info.balance, balance);
    assert_eq!(caller.info.nonce, 0);

    Ok(())
}

/// Before TsukiV2 the mainnet nonce check runs before the L1 fee needs the RLP bytes; from
/// TsukiV2 the L1 fee is computed first.
#[rstest]
#[case::galileo(ScrollSpecId::GALILEO)]
#[case::tsuki(ScrollSpecId::TSUKI)]
#[case::tsuki_v2(ScrollSpecId::TSUKI_V2)]
fn bad_nonce_and_missing_rlp(#[case] spec: ScrollSpecId) {
    let ctx = transfer(spec, gas_cost(BASE_FEE) + L1_DATA_COST, BASE_FEE.into(), U256::ZERO)
        .modify_tx_chained(|tx| {
            tx.base.nonce = 1;
            tx.rlp_bytes = None;
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let expected = if spec.is_enabled_in(ScrollSpecId::TSUKI_V2) {
        EVMError::Custom("[SCROLL] Failed to load transaction rlp_bytes.".to_string())
    } else {
        EVMError::Transaction(InvalidTransaction::NonceTooHigh { tx: 1, state: 0 })
    };
    assert_eq!(handler.pre_execution(&mut evm), Err(expected));
}

/// EIP-3607 rejects a sender with deployed code in the mainnet check. Before TsukiV2 that check
/// runs first; from TsukiV2 an unfunded sender fails the combined balance check first.
#[rstest]
#[case::galileo(ScrollSpecId::GALILEO)]
#[case::tsuki(ScrollSpecId::TSUKI)]
#[case::tsuki_v2(ScrollSpecId::TSUKI_V2)]
fn sender_with_code(#[case] spec: ScrollSpecId) {
    let required = gas_cost(BASE_FEE) + L1_DATA_COST;
    for balance in [U256::ZERO, required] {
        let code = Bytecode::new_legacy(Bytes::from_static(&[STOP]));
        let ctx = transfer(spec, balance, BASE_FEE.into(), U256::ZERO).modify_db_chained(|db| {
            db.insert_account_info(CALLER, AccountInfo::new(balance, 0, code.hash_slow(), code))
        });
        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

        let expected = if balance.is_zero() && spec.is_enabled_in(ScrollSpecId::TSUKI_V2) {
            InvalidTransaction::LackOfFundForMaxFee {
                fee: Box::new(required),
                balance: Box::new(U256::ZERO),
            }
        } else {
            InvalidTransaction::RejectCallerWithCode
        };
        assert_eq!(handler.pre_execution(&mut evm), Err(EVMError::Transaction(expected)));
    }
}

/// A CREATE at the boundary balance is accepted before TsukiV2 and rejected from it.
#[rstest]
#[case::tsuki(ScrollSpecId::TSUKI)]
#[case::tsuki_v2(ScrollSpecId::TSUKI_V2)]
fn boundary_balance_create(#[case] spec: ScrollSpecId) -> Result<(), Box<dyn core::error::Error>> {
    let (gas_limit, max_fee, value) = (53_000u64, 10u64, U256::from(100));
    let max_gas = U256::from(gas_limit) * U256::from(max_fee);
    let gas = U256::from(gas_limit) * U256::from(BASE_FEE);
    let balance = max_gas + value;
    assert!(gas + L1_DATA_COST <= balance);

    let ctx = tx(spec, TxKind::Create, gas_limit, balance, max_fee.into(), value);
    let transaction = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    if spec.is_enabled_in(ScrollSpecId::TSUKI_V2) {
        assert_eq!(
            evm.transact(transaction).unwrap_err(),
            EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
                fee: Box::new(max_gas + value + L1_DATA_COST),
                balance: Box::new(balance),
            })
        );
    } else {
        let result = evm.transact(transaction)?;
        assert!(result.result.is_success());
        let caller = &result.state[&CALLER].info;
        assert_eq!(caller.balance, balance - gas - L1_DATA_COST - value);
        assert_eq!(caller.nonce, 1);
    }

    Ok(())
}

/// Enough balance for the combined check passes at both specs with identical charges.
#[rstest]
#[case(ScrollSpecId::TSUKI)]
#[case(ScrollSpecId::TSUKI_V2)]
fn exact_combined_balance(#[case] spec: ScrollSpecId) -> Result<(), Box<dyn core::error::Error>> {
    let max_fee = 10;
    let value = U256::from(100);
    let balance = gas_cost(max_fee) + value + L1_DATA_COST;
    let ctx = transfer(spec, balance, max_fee.into(), value);
    let tx = ctx.tx.clone();
    let result = ctx.build_scroll().transact(tx)?;
    assert!(result.result.is_success());
    let caller = &result.state[&CALLER].info;
    assert_eq!(caller.balance, balance - gas_cost(BASE_FEE) - L1_DATA_COST - value);
    assert_eq!(caller.nonce, 1);

    Ok(())
}
