use crate::{
    builder::ScrollBuilder,
    gas::scroll_gas_params,
    handler::ScrollHandler,
    l1block::L1BlockInfo,
    test_utils::{
        context, ScrollContextTestUtils, BENEFICIARY, CALLER, L1_DATA_COST, MIN_TRANSACTION_COST,
    },
    transaction::L1_MESSAGE_TYPE,
    ScrollSpecId,
};
use std::boxed::Box;

use revm::{
    context::{
        journaled_state::account::JournaledAccountTr,
        result::{EVMError, ExecutionResult, HaltReason, InvalidTransaction, ResultAndState},
        ContextTr, JournalTr, TransactionType,
    },
    handler::{EthFrame, EvmTr, FrameResult, Handler},
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    state::Bytecode,
    ExecuteEvm,
};
use revm_primitives::{bytes, U256};

#[test]
fn test_l1_message_validate_lacking_funds() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    // pre execution includes fees deduction, which should be skipped for l1 messages.
    let mut init_and_floor_gas = handler.validate(&mut evm)?;
    handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

    Ok(())
}

#[test]
fn test_l1_message_load_accounts() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    handler.load_accounts(&mut evm)?;

    // l1 block info should not be loaded for l1 messages.
    let l1_block_info = evm.ctx().chain.l1_block_info.clone();
    assert_eq!(l1_block_info, L1BlockInfo::default());

    Ok(())
}

#[test]
fn test_l1_message_should_not_deduct_caller() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);

    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;
    handler.load_accounts(&mut evm)?;
    handler.validate_against_state_and_deduct_caller(&mut evm, &mut init_and_floor_gas)?;

    // nonce should be increase and caller should have same balance as the start (0).
    let ctx = evm.ctx_mut();
    let caller_account = ctx.journal_mut().load_account(CALLER)?;
    assert_eq!(caller_account.info.balance, U256::ZERO);
    assert_eq!(caller_account.info.nonce, 1);

    Ok(())
}

#[test]
fn test_l1_message_last_frame_result() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);

    let mut evm = ctx.build_scroll();
    let mut handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut gas = Gas::new(21000);
    gas.set_refund(10);
    gas.set_spent(10);
    let mut result = FrameResult::Call(CallOutcome::new(
        InterpreterResult { result: InstructionResult::Return, output: Default::default(), gas },
        0..0,
    ));
    handler.last_frame_result(&mut evm, 0, &mut result)?;

    // refund should be 0 for l1 messages.
    gas.set_refund(0);
    assert_eq!(result.gas(), &gas);

    Ok(())
}

#[test]
fn test_l1_message_should_not_refund() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);

    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut gas = Gas::new(21000);
    gas.set_refund(10);
    gas.set_spent(10);
    let mut result = FrameResult::Call(CallOutcome::new(
        InterpreterResult { result: InstructionResult::Return, output: Default::default(), gas },
        0..0,
    ));
    handler.refund(&mut evm, &mut result, 0);

    // gas should not have been updated
    assert_eq!(result.gas(), &gas);

    Ok(())
}

#[test]
fn test_l1_message_should_not_reward_beneficiary() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| tx.base.tx_type = L1_MESSAGE_TYPE);

    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let gas = Gas::new_spent_with_reservoir(21000, 0);
    let mut result = FrameResult::Call(CallOutcome::new(
        InterpreterResult { result: InstructionResult::Return, output: Default::default(), gas },
        0..0,
    ));
    handler.load_accounts(&mut evm)?;
    handler.reward_beneficiary(&mut evm, &mut result)?;

    // beneficiary should not see his balance increased for l1 message execution.
    let ctx = evm.ctx_mut();
    let beneficiary = ctx.journal_mut().load_account(BENEFICIARY)?;
    assert_eq!(beneficiary.info.balance, U256::ZERO);

    Ok(())
}

#[test]
fn test_l1_message_should_revert_with_out_of_funds() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context().modify_tx_chained(|tx| {
        tx.base.tx_type = L1_MESSAGE_TYPE;
        tx.base.value = U256::ONE;
    });
    let tx = ctx.tx.clone();
    let mut evm = ctx.build_scroll();

    let ResultAndState { result, .. } = evm.transact(tx)?;

    // L1 message should pass pre-execution but revert with `OutOfFunds`.
    match result {
        ExecutionResult::Halt { reason: HaltReason::OutOfFunds, gas, logs } => {
            assert_eq!(gas.tx_gas_used(), MIN_TRANSACTION_COST.to());
            assert!(logs.is_empty());
        }
        result => panic!("expected OutOfFunds halt, got {result:?}"),
    }

    Ok(())
}

#[test]
fn test_l1_message_should_pass_validation() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context()
        .modify_tx_chained(|tx| {
            tx.base.tx_type = L1_MESSAGE_TYPE;
            tx.base.value = U256::ONE;
            tx.base.gas_price = 0;
        })
        // set the base fee of the block above the L1 message gas price to check it passes.
        .modify_block_chained(|block| block.basefee = 100);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    handler.validate(&mut evm)?;

    Ok(())
}

#[test]
fn test_l1_message_validate_env_keeps_custom_type() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context()
        .modify_tx_chained(|tx| {
            tx.base.tx_type = L1_MESSAGE_TYPE;
            tx.base.chain_id = None;
            tx.base.gas_price = 0;
        })
        .modify_block_chained(|block| block.basefee = 100);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    assert_eq!(TransactionType::from(L1_MESSAGE_TYPE), TransactionType::Custom);
    handler.validate_env(&mut evm)?;

    Ok(())
}

#[test]
fn test_l1_message_should_pass_pre_execution() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context()
        .modify_tx_chained(|tx| {
            tx.base.tx_type = L1_MESSAGE_TYPE;
        })
        // set the caller nonce to 1 and check pre execution passes.
        .modify_journal_chained(|journal| {
            let mut caller = journal.load_account_mut(CALLER).unwrap();
            caller.data.set_nonce(1);
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let mut init_and_floor_gas = handler.validate(&mut evm)?;
    handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

    Ok(())
}

#[test]
fn test_l1_data_fee_buffer_default_accepts_1x() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context()
        .with_scroll_spec(ScrollSpecId::CURIE)
        .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;

    handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    assert_eq!(caller.data.info.balance, U256::ZERO);
    assert_eq!(caller.data.info.nonce, 1);

    Ok(())
}

#[test]
fn test_l1_data_fee_buffer_policy_rejects_below_2x() -> Result<(), Box<dyn core::error::Error>> {
    let required_l1_balance = L1_DATA_COST + L1_DATA_COST;
    let ctx = context()
        .with_scroll_spec(ScrollSpecId::CURIE)
        .with_funds(MIN_TRANSACTION_COST + required_l1_balance - U256::ONE)
        .with_l1_data_fee_buffer(true);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;

    let err = handler.pre_execution(&mut evm, &mut init_and_floor_gas).unwrap_err();
    assert_eq!(
        err,
        EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
            fee: Box::new(required_l1_balance),
            balance: Box::new(required_l1_balance - U256::ONE),
        })
    );

    Ok(())
}

#[test]
fn test_l1_data_fee_buffer_policy_accepts_boundary() -> Result<(), Box<dyn core::error::Error>> {
    let required_l1_balance = L1_DATA_COST + L1_DATA_COST;
    let ctx = context()
        .with_scroll_spec(ScrollSpecId::CURIE)
        .with_funds(MIN_TRANSACTION_COST + required_l1_balance)
        .with_l1_data_fee_buffer(true);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;

    handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

    let caller = evm.ctx_mut().journal_mut().load_account(CALLER)?;
    assert_eq!(caller.data.info.balance, L1_DATA_COST);
    assert_eq!(caller.data.info.nonce, 1);

    Ok(())
}

#[test]
fn test_l1_data_fee_buffer_policy_rewards_1x() -> Result<(), Box<dyn core::error::Error>> {
    let required_l1_balance = L1_DATA_COST + L1_DATA_COST;
    let ctx = context()
        .with_scroll_spec(ScrollSpecId::CURIE)
        .with_funds(MIN_TRANSACTION_COST + required_l1_balance)
        .with_l1_data_fee_buffer(true);
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;
    let gas = Gas::new_spent_with_reservoir(21000, 0);
    let mut result = FrameResult::Call(CallOutcome::new(
        InterpreterResult { result: InstructionResult::Return, output: Default::default(), gas },
        0..0,
    ));

    handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;
    handler.reward_beneficiary(&mut evm, &mut result)?;

    let beneficiary = evm.ctx_mut().journal_mut().load_account(BENEFICIARY)?;
    assert_eq!(beneficiary.data.info.balance, MIN_TRANSACTION_COST + L1_DATA_COST);

    Ok(())
}

#[test]
fn test_l1_message_eip_3607() -> Result<(), Box<dyn core::error::Error>> {
    let ctx = context()
        .modify_tx_chained(|tx| {
            tx.base.tx_type = L1_MESSAGE_TYPE;
        })
        // set the caller nonce to 1 and check pre execution passes.
        .modify_journal_chained(|journal| {
            let mut caller = journal.load_account_mut(CALLER).unwrap();
            caller.data.set_code_and_hash_slow(Bytecode::new_raw(bytes!("0x0101")));
        });
    let mut evm = ctx.build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
    let mut init_and_floor_gas = handler.validate(&mut evm)?;

    let err = handler.pre_execution(&mut evm, &mut init_and_floor_gas).unwrap_err();
    assert_eq!(err, EVMError::Transaction(InvalidTransaction::RejectCallerWithCode));

    Ok(())
}

#[test]
fn test_l1_message_should_not_have_floor_gas_as_gas_used() -> Result<(), Box<dyn core::error::Error>>
{
    let ctx = context().with_scroll_spec(ScrollSpecId::FEYNMAN).modify_tx_chained(|tx| {
        tx.base.data =
            bytes!("0x000000000123456789abcdef00000000123456789abcdef00000000123456789abcdef");
        tx.base.tx_type = L1_MESSAGE_TYPE;
        tx.base.caller = CALLER;
        tx.base.gas_limit = 200000;
        tx.base.value = U256::ONE;
    });
    let tx = ctx.tx.clone();
    let mut evm = ctx.build_scroll();
    let res = evm.transact(tx.clone())?;

    // floor gas is TOTAL_COST_FLOOR_PER_TOKEN * tokens_in_calldata + 21_000 = 22070;
    let expected_init_gas =
        scroll_gas_params(ScrollSpecId::FEYNMAN).initial_tx_gas_for_tx(&tx).initial_regular_gas();

    match res.result {
        ExecutionResult::Halt { reason: HaltReason::OutOfFunds, gas, logs } => {
            assert_eq!(gas.tx_gas_used(), expected_init_gas);
            assert!(logs.is_empty());
        }
        result => panic!("expected OutOfFunds halt, got {result:?}"),
    }

    Ok(())
}

#[test]
fn test_l1_message_still_validates_eip7623_floor_gas() {
    let calldata =
        bytes!("0x000000000123456789abcdef00000000123456789abcdef00000000123456789abcdef");
    let ctx = context().with_scroll_spec(ScrollSpecId::FEYNMAN).modify_tx_chained(|tx| {
        tx.base.data = calldata.clone();
        tx.base.tx_type = L1_MESSAGE_TYPE;
        tx.base.gas_limit = 200000;
    });
    let initial_gas = scroll_gas_params(ScrollSpecId::FEYNMAN).initial_tx_gas_for_tx(&ctx.tx);
    let gas_limit = initial_gas.initial_regular_gas();
    assert!(gas_limit < initial_gas.floor_gas());

    let mut evm = ctx.modify_tx_chained(|tx| tx.base.gas_limit = gas_limit).build_scroll();
    let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();

    let err = handler.validate_initial_tx_gas(&mut evm).unwrap_err();
    assert_eq!(
        err,
        EVMError::Transaction(InvalidTransaction::GasFloorMoreThanGasLimit {
            gas_limit,
            gas_floor: initial_gas.floor_gas(),
        })
    );
}
