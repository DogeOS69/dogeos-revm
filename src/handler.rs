//! Handler related to Scroll chain.

use crate::{exec::ScrollContextTr, l1block::L1BlockInfo, transaction::ScrollTxTr, ScrollSpecId};
use std::{boxed::Box, string::ToString};

use revm::{
    context::{
        journaled_state::account::JournaledAccountTr,
        result::{HaltReason, InvalidHeader, InvalidTransaction, ResultGas},
        transaction::TransactionType,
        Block, Cfg, ContextTr, JournalTr, Transaction,
    },
    handler::{
        post_execution, pre_execution, validation, EthFrame, EvmTr, EvmTrError, FrameResult,
        FrameTr, Handler, MainnetHandler,
    },
    interpreter::{
        interpreter::EthInterpreter, interpreter_action::FrameInit, Gas, InitialAndFloorGas,
    },
    primitives::{hardfork::SpecId, U256},
};
use revm_inspector::{Inspector, InspectorEvmTr, InspectorHandler};

/// The Scroll handler.
pub struct ScrollHandler<EVM, ERROR, FRAME> {
    pub mainnet: MainnetHandler<EVM, ERROR, FRAME>,
}

impl<EVM, ERROR, FRAME> ScrollHandler<EVM, ERROR, FRAME> {
    pub fn new() -> Self {
        Self { mainnet: MainnetHandler::default() }
    }
}

impl<EVM, ERROR, FRAME> Default for ScrollHandler<EVM, ERROR, FRAME> {
    fn default() -> Self {
        Self::new()
    }
}

/// Configure the handler for the Scroll chain.
///
/// The trait modifies the following handlers:
/// - `pre_execution` - Adds a hook to load `L1BlockInfo` from the database such that it can be used
///   to calculate the L1 cost of a transaction.
/// - `validate_against_state_and_deduct_caller` - Overrides the logic to deduct the max transaction
///   fee, including the L1 fee, from the caller's balance.
/// - `last_frame_result` - Overrides the logic for gas refund in the case the transaction is a L1
///   message.
/// - `refund` - Overrides the logic for gas refund in the case the transaction is a L1 message.
/// - `post_execution.reward_beneficiary` - Overrides the logic to reward the beneficiary with the
///   gas fee and skip rewarding in case the transaction is a L1 message.
impl<EVM, ERROR, FRAME> Handler for ScrollHandler<EVM, ERROR, FRAME>
where
    EVM: EvmTr<Context: ScrollContextTr, Frame = FRAME>,
    ERROR: EvmTrError<EVM> + From<InvalidTransaction>,
    FRAME: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = HaltReason;

    #[inline]
    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        // Mirrors revm-handler 41.0.0 environment validation. Deliberate Scroll delta:
        // EIP-7702 is accepted for Euclid+ while the Ethereum base spec stays Shanghai,
        // so EIP-4844 and later mainnet envelopes remain rejected.
        let ctx = evm.ctx_ref();
        let eth_spec: SpecId = ctx.cfg().spec().into();

        if eth_spec.is_enabled_in(SpecId::MERGE) && ctx.block().prevrandao().is_none() {
            return Err(InvalidHeader::PrevrandaoNotSet.into());
        }
        if eth_spec.is_enabled_in(SpecId::CANCUN)
            && ctx.block().blob_excess_gas_and_price().is_none()
        {
            return Err(InvalidHeader::ExcessBlobGasNotSet.into());
        }

        let tx = ctx.tx();
        let tx_type = TransactionType::from(tx.tx_type());
        let base_fee = if ctx.cfg().is_base_fee_check_disabled() {
            None
        } else {
            Some(ctx.block().basefee() as u128)
        };

        if ctx.cfg().tx_chain_id_check() {
            if let Some(chain_id) = tx.chain_id() {
                if chain_id != ctx.cfg().chain_id() {
                    return Err(InvalidTransaction::InvalidChainId.into());
                }
            } else if !tx_type.is_legacy() && !tx_type.is_custom() {
                return Err(InvalidTransaction::MissingChainId.into());
            }
        }

        // Scroll's Ethereum base spec is Shanghai, so EIP-8037 is normally disabled unless
        // tests or explicit config enable it.
        if !ctx.cfg().is_amsterdam_eip8037_enabled() {
            let cap = ctx.cfg().tx_gas_limit_cap();
            if tx.gas_limit() > cap {
                return Err(InvalidTransaction::TxGasLimitGreaterThanCap {
                    gas_limit: tx.gas_limit(),
                    cap,
                }
                .into());
            }
        }

        let disable_priority_fee_check = ctx.cfg().is_priority_fee_check_disabled();
        let validate_priority_fee = || {
            validation::validate_priority_fee_tx(
                tx.max_fee_per_gas(),
                tx.max_priority_fee_per_gas().unwrap_or_default(),
                base_fee,
                disable_priority_fee_check,
            )
        };
        match tx_type {
            TransactionType::Legacy => {
                validation::validate_legacy_gas_price(tx.gas_price(), base_fee)?;
            }
            TransactionType::Eip2930 => {
                if !eth_spec.is_enabled_in(SpecId::BERLIN) {
                    return Err(InvalidTransaction::Eip2930NotSupported.into());
                }
                validation::validate_legacy_gas_price(tx.gas_price(), base_fee)?;
            }
            TransactionType::Eip1559 => {
                if !eth_spec.is_enabled_in(SpecId::LONDON) {
                    return Err(InvalidTransaction::Eip1559NotSupported.into());
                }
                validate_priority_fee()?;
            }
            TransactionType::Eip4844 => {
                if !eth_spec.is_enabled_in(SpecId::CANCUN) {
                    return Err(InvalidTransaction::Eip4844NotSupported.into());
                }
                validate_priority_fee()?;
                validation::validate_eip4844_tx(
                    tx.blob_versioned_hashes(),
                    tx.max_fee_per_blob_gas(),
                    ctx.block().blob_gasprice().unwrap_or_default(),
                    ctx.cfg().max_blobs_per_tx(),
                )?;
            }
            TransactionType::Eip7702 => {
                if !ctx.cfg().spec().is_enabled_in(ScrollSpecId::EUCLID) {
                    return Err(InvalidTransaction::Eip7702NotSupported.into());
                }
                validate_priority_fee()?;
                if tx.authorization_list_len() == 0 {
                    return Err(InvalidTransaction::EmptyAuthorizationList.into());
                }
            }
            TransactionType::Custom => {}
        }

        if !ctx.cfg().is_block_gas_limit_disabled() && tx.gas_limit() > ctx.block().gas_limit() {
            return Err(InvalidTransaction::CallerGasLimitMoreThanBlock.into());
        }

        if eth_spec.is_enabled_in(SpecId::SHANGHAI)
            && tx.kind().is_create()
            && tx.input().len() > ctx.cfg().max_initcode_size()
        {
            return Err(InvalidTransaction::CreateInitCodeSizeLimit.into());
        }

        if tx.nonce() == u64::MAX {
            return Err(InvalidTransaction::NonceOverflowInTransaction.into());
        }

        Ok(())
    }

    #[inline]
    fn validate_initial_tx_gas(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<InitialAndFloorGas, Self::Error> {
        let ctx = evm.ctx_ref();
        let tx = ctx.tx();
        let mut gas = ctx.cfg().gas_params().initial_tx_gas_for_tx(tx);
        let is_eip8037 = ctx.cfg().is_amsterdam_eip8037_enabled();

        if ctx.cfg().is_eip7623_disabled() {
            gas.set_floor_gas(0);
        }
        if !is_eip8037 {
            gas.set_initial_state_gas(0);
        }

        if gas.initial_total_gas() > tx.gas_limit() {
            return Err(InvalidTransaction::CallGasCostMoreThanGasLimit {
                gas_limit: tx.gas_limit(),
                initial_gas: gas.initial_total_gas(),
            }
            .into());
        }

        if gas.floor_gas() > 0 && gas.floor_gas() > tx.gas_limit() {
            return Err(InvalidTransaction::GasFloorMoreThanGasLimit {
                gas_floor: gas.floor_gas(),
                gas_limit: tx.gas_limit(),
            }
            .into());
        }

        let cap = ctx.cfg().tx_gas_limit_cap();
        if is_eip8037 && tx.gas_limit() > cap {
            let min_regular_gas = gas.initial_regular_gas().max(gas.floor_gas());
            if min_regular_gas > cap {
                return Err(InvalidTransaction::GasFloorMoreThanGasLimit {
                    gas_floor: min_regular_gas,
                    gas_limit: cap,
                }
                .into());
            }
        }

        Ok(gas)
    }

    #[inline]
    fn pre_execution(
        &self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &mut InitialAndFloorGas,
    ) -> Result<u64, Self::Error> {
        // only load the L1BlockInfo for txs that are not l1 messages.
        if !evm.ctx().tx().is_l1_msg() && !evm.ctx().tx().is_system_tx() {
            let spec = evm.ctx().cfg().spec();
            let l1_block_info = L1BlockInfo::try_fetch(evm.ctx().db_mut(), spec)?;
            evm.ctx().chain_mut().l1_block_info = l1_block_info;
        }

        self.validate_against_state_and_deduct_caller(evm, init_and_floor_gas)?;
        self.load_accounts(evm)?;
        let gas = self.apply_eip7702_auth_list(evm, init_and_floor_gas)?;
        Ok(gas)
    }

    #[inline]
    fn apply_eip7702_auth_list(
        &self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &mut InitialAndFloorGas,
    ) -> Result<u64, Self::Error> {
        let ctx = evm.ctx_mut();
        if ctx.tx().tx_type() != TransactionType::Eip7702 {
            return Ok(0);
        }
        if !ctx.cfg().spec().is_enabled_in(ScrollSpecId::EUCLID) {
            return Ok(0);
        }

        let chain_id = ctx.cfg().chain_id();
        let is_eip8037 = ctx.cfg().is_amsterdam_eip8037_enabled();
        let params = ctx.cfg().gas_params().clone();
        let (tx, journal) = ctx.tx_journal_mut();
        let (refunded_accounts, refunded_bytecodes) = pre_execution::apply_auth_list::<
            _,
            Self::Error,
        >(
            chain_id, tx.authorization_list(), journal
        )?;

        if is_eip8037 {
            init_and_floor_gas.state_refund +=
                params.tx_eip7702_state_refund(refunded_accounts, refunded_bytecodes);
        }

        Ok(params.tx_eip7702_auth_refund_regular().saturating_mul(refunded_accounts))
    }

    #[inline]
    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &mut InitialAndFloorGas,
    ) -> Result<(), Self::Error> {
        // load caller's account.
        let ctx_ref = evm.ctx_ref();
        let caller = ctx_ref.tx().caller();
        let is_l1_msg = ctx_ref.tx().is_l1_msg();
        let is_system_tx = ctx_ref.tx().is_system_tx();
        let spec = ctx_ref.cfg().spec();
        let is_eip3607_disabled = ctx_ref.cfg().is_eip3607_disabled();

        // execute normal checks and transaction processing logic for non-l1-msgs
        if !is_l1_msg {
            // The mainnet handler checks and deducts the maximum transaction cost before Scroll
            // applies the additional L1 data fee below.
            self.mainnet.validate_against_state_and_deduct_caller(evm, init_and_floor_gas)?;
        }

        // process rollup fee
        let ctx = evm.ctx();
        if !is_l1_msg && !is_system_tx {
            let l1_block_info = ctx.chain().l1_block_info.clone();
            let Some(rlp_bytes) = ctx.tx().rlp_bytes() else {
                return Err(ERROR::from_string(
                    "[SCROLL] Failed to load transaction rlp_bytes.".to_string(),
                ));
            };

            // Deduct l1 fee from caller.
            let tx_l1_cost = l1_block_info.calculate_tx_l1_cost(
                rlp_bytes,
                spec,
                ctx.tx().compression_ratio(),
                ctx.tx().compressed_size(),
            );

            // Optionally check balance covers 2x L1 cost (1 unit charged + 1 unit buffer)
            let l1_cost_with_optional_buffer = if ctx.chain().policy.require_l1_data_fee_buffer {
                tx_l1_cost.saturating_add(tx_l1_cost)
            } else {
                tx_l1_cost
            };

            let mut caller_account = ctx.journal_mut().load_account_mut(caller)?;

            // Ensure caller has enough balance to cover L1 cost + optional buffer
            let caller_balance = *caller_account.data.balance();
            if l1_cost_with_optional_buffer.gt(&caller_balance) {
                return Err(InvalidTransaction::LackOfFundForMaxFee {
                    fee: l1_cost_with_optional_buffer.into(),
                    balance: caller_balance.into(),
                }
                .into());
            }

            // Deduct only actual L1 cost (buffer is NOT deducted)
            caller_account.data.set_balance(caller_balance.saturating_sub(tx_l1_cost));
        }

        // execute l1 msg checks
        if is_l1_msg {
            // Load caller's account.
            let (tx, journal) = ctx.tx_journal_mut();
            let mut caller_account = journal.load_account_with_code_mut(caller)?;

            // Note: we skip the balance check at pre-execution level if the transaction is a
            // L1 message and Euclid is enabled. This means the L1 message will reach execution
            // stage in revm and revert with `OutOfFunds` in the first frame, but still be included
            // in the block.
            let skip_balance_check = tx.is_l1_msg() && spec.is_enabled_in(ScrollSpecId::EUCLID);
            if !skip_balance_check {
                let max_balance_spending = tx.max_balance_spending()?;
                let caller_balance = *caller_account.data.balance();
                if max_balance_spending > caller_balance {
                    return Err(InvalidTransaction::LackOfFundForMaxFee {
                        fee: Box::new(max_balance_spending),
                        balance: Box::new(caller_balance),
                    }
                    .into());
                }
            }

            // EIP-3607: Reject transactions from senders with deployed code.
            //
            // We check the sender of the L1 message is a EOA on the L2.
            // If the sender is a (delegated) EOA on the L1, it should be a (delegated) EOA
            // on the L2.
            // If the sender is a contract on the L1, address aliasing assures with high probability
            // that the L2 sender would be an EOA.
            if !is_eip3607_disabled {
                // Allow EOAs whose code is a valid delegation designation,
                // i.e. 0xef0100 || address, to continue to originate transactions.
                if let Some(bytecode) = caller_account.data.code() {
                    if !bytecode.is_empty() && !bytecode.is_eip7702() {
                        return Err(InvalidTransaction::RejectCallerWithCode.into());
                    }
                }
            }

            // Bump the nonce for calls. Nonce for CREATE will be bumped in `make_create_frame`.
            if tx.kind().is_call() {
                // Nonce is already checked
                caller_account.data.bump_nonce();
            }
            // touch account so we know it is changed.
            caller_account.data.touch();
        }
        Ok(())
    }

    #[inline]
    fn last_frame_result(
        &mut self,
        evm: &mut Self::Evm,
        original_reservoir: u64,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        if !evm.ctx().tx().is_l1_msg() {
            return self.mainnet.last_frame_result(evm, original_reservoir, frame_result);
        }

        let instruction_result = frame_result.interpreter_result().result;
        let create_failed =
            matches!(frame_result, FrameResult::Create(_)) && !instruction_result.is_ok();
        let create_state_gas_refund =
            if create_failed && evm.ctx().cfg().is_amsterdam_eip8037_enabled() {
                Some(evm.ctx().cfg().gas_params().create_state_gas())
            } else {
                None
            };
        let gas = frame_result.gas_mut();
        let remaining = gas.remaining();
        let reservoir = gas.reservoir();
        let state_gas_spent = gas.state_gas_spent();

        // Spend the gas limit first. Successful and reverted L1 messages recover unused regular
        // gas below; halts keep the regular gas spent.
        *gas = Gas::new_spent_with_reservoir(evm.ctx().tx().gas_limit(), reservoir);

        if instruction_result.is_ok_or_revert() {
            gas.erase_cost(remaining);
        }

        if instruction_result.is_ok() {
            gas.set_state_gas_spent(state_gas_spent);
        } else {
            gas.set_state_gas_spent(0);
            gas.set_reservoir(reservoir.saturating_add_signed(state_gas_spent));
        }

        if let Some(state_gas_charged) = create_state_gas_refund {
            gas.refill_reservoir(state_gas_charged);
        }

        Ok(())
    }

    #[inline]
    fn eip7623_check_gas_floor(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
        init_and_floor_gas: InitialAndFloorGas,
    ) {
        // skip floor gas check for l1 messages.
        if evm.ctx().tx().is_l1_msg() {
            return;
        }
        self.mainnet.eip7623_check_gas_floor(evm, exec_result, init_and_floor_gas)
    }

    #[inline]
    fn post_execution(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut FrameResult,
        init_and_floor_gas: InitialAndFloorGas,
        eip7702_refund: i64,
    ) -> Result<ResultGas, Self::Error> {
        self.refund(evm, exec_result, eip7702_refund);

        let mut result_gas_init = init_and_floor_gas;
        if evm.ctx().tx().is_l1_msg() {
            result_gas_init.set_floor_gas(0);
        }
        let result_gas = post_execution::build_result_gas(
            exec_result.instruction_result().is_halt(),
            exec_result.gas(),
            result_gas_init,
        );

        self.eip7623_check_gas_floor(evm, exec_result, init_and_floor_gas);
        self.reimburse_caller(evm, exec_result)?;
        self.reward_beneficiary(evm, exec_result)?;
        Ok(result_gas)
    }

    #[inline]
    fn refund(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
        eip7702_refund: i64,
    ) {
        // skip refund for l1 messages
        if evm.ctx().tx().is_l1_msg() {
            return;
        }
        let spec = evm.ctx().cfg().spec().into();
        post_execution::refund(spec, exec_result.gas_mut(), eip7702_refund)
    }

    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        let ctx = evm.ctx();

        // If the transaction is an L1 message, we do not need to reward the beneficiary as the
        // transaction has already been paid for on L1.
        if ctx.tx().is_l1_msg() {
            return Ok(());
        }

        // fetch the effective gas price.
        let block = ctx.block();
        let effective_gas_price = U256::from(ctx.tx().effective_gas_price(block.basefee() as u128));

        // load beneficiary's account.
        let beneficiary = block.beneficiary();

        // calculate the L1 cost of the transaction.
        let l1_cost = if !ctx.tx().is_system_tx() {
            let l1_block_info = ctx.chain().l1_block_info.clone();
            let Some(rlp_bytes) = &ctx.tx().rlp_bytes() else {
                return Err(ERROR::from_string(
                    "[SCROLL] Failed to load transaction rlp_bytes.".to_string(),
                ));
            };
            l1_block_info.calculate_tx_l1_cost(
                rlp_bytes,
                ctx.cfg().spec(),
                ctx.tx().compression_ratio(),
                ctx.tx().compressed_size(),
            )
        } else {
            U256::from(0)
        };

        // reward the beneficiary with the gas fee including the L1 cost of the transaction and mark
        // the account as touched.
        let gas = exec_result.gas();

        let gas_used = gas.used().saturating_sub(gas.reservoir());
        let reward =
            effective_gas_price.saturating_mul(U256::from(gas_used)).saturating_add(l1_cost);
        ctx.journal_mut().balance_incr(beneficiary, reward)?;

        Ok(())
    }
}

impl<EVM, ERROR> InspectorHandler for ScrollHandler<EVM, ERROR, EthFrame<EthInterpreter>>
where
    EVM: InspectorEvmTr<
        Context: ScrollContextTr,
        Frame = EthFrame<EthInterpreter>,
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
    >,
    ERROR: EvmTrError<EVM>,
{
    type IT = EthInterpreter;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builder::ScrollBuilder,
        test_utils::{
            context, ScrollContextTestUtils, BENEFICIARY, CALLER, L1_DATA_COST,
            MIN_TRANSACTION_COST,
        },
        ScrollSpecId,
    };
    use std::boxed::Box;

    use revm::{
        context::result::EVMError,
        handler::EthFrame,
        interpreter::{CallOutcome, InstructionResult, InterpreterResult},
    };

    #[test]
    fn test_validate_lacking_funds() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context();
        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate_initial_tx_gas(&mut evm)?;
        let err = handler
            .validate_against_state_and_deduct_caller(&mut evm, &mut init_and_floor_gas)
            .unwrap_err();
        assert_eq!(
            err,
            EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee {
                fee: Box::new(U256::from(21000)),
                balance: Box::default()
            })
        );

        Ok(())
    }

    #[test]
    fn test_load_account() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);
        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

        let l1_block_info = evm.ctx().chain.l1_block_info.clone();
        assert_ne!(l1_block_info, L1BlockInfo::default());

        Ok(())
    }

    #[test]
    fn test_deduct_caller() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);

        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

        let ctx = evm.ctx_mut();
        let caller_account = ctx.journal_mut().load_account(CALLER)?;
        assert_eq!(caller_account.info.balance, U256::ZERO);
        assert_eq!(caller_account.info.nonce, 1);

        Ok(())
    }

    #[test]
    fn test_last_frame_result() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context();

        let mut evm = ctx.build_scroll();
        let mut handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut gas = Gas::new(21000);
        gas.set_refund(10);
        gas.set_spent(10);
        let mut result = FrameResult::Call(CallOutcome::new(
            InterpreterResult {
                result: InstructionResult::Return,
                output: Default::default(),
                gas,
            },
            0..0,
        ));
        handler.last_frame_result(&mut evm, 0, &mut result)?;

        assert_eq!(result.gas(), &gas);

        Ok(())
    }

    #[test]
    fn test_refund() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context();

        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut gas = Gas::new(21000);
        gas.set_refund(10);
        gas.set_spent(10);
        let mut result = FrameResult::Call(CallOutcome::new(
            InterpreterResult {
                result: InstructionResult::Return,
                output: Default::default(),
                gas,
            },
            0..0,
        ));
        handler.refund(&mut evm, &mut result, 0);

        gas.set_refund(2);
        assert_eq!(result.gas(), &gas);

        Ok(())
    }

    #[test]
    fn test_reward_beneficiary() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);

        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let gas = Gas::new_spent_with_reservoir(21000, 0);
        let mut result = FrameResult::Call(CallOutcome::new(
            InterpreterResult {
                result: InstructionResult::Return,
                output: Default::default(),
                gas,
            },
            0..0,
        ));
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;
        handler.reward_beneficiary(&mut evm, &mut result)?;

        let ctx = evm.ctx_mut();
        let beneficiary = ctx.journal_mut().load_account(BENEFICIARY)?;
        assert_eq!(beneficiary.info.balance, MIN_TRANSACTION_COST + L1_DATA_COST);

        Ok(())
    }

    #[test]
    fn test_reward_beneficiary_subtracts_reservoir() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);

        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let gas = Gas::new_spent_with_reservoir(21000, 5000);
        let mut result = FrameResult::Call(CallOutcome::new(
            InterpreterResult {
                result: InstructionResult::Return,
                output: Default::default(),
                gas,
            },
            0..0,
        ));
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;
        handler.reward_beneficiary(&mut evm, &mut result)?;

        let beneficiary = evm.ctx_mut().journal_mut().load_account(BENEFICIARY)?;
        assert_eq!(beneficiary.info.balance, U256::from(16000) + L1_DATA_COST);

        Ok(())
    }

    #[test]
    fn test_transaction_pre_execution() -> Result<(), Box<dyn core::error::Error>> {
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);

        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        handler.pre_execution(&mut evm, &mut init_and_floor_gas)?;

        Ok(())
    }

    #[test]
    fn test_validate_l1_cost_buffer_required() -> Result<(), Box<dyn core::error::Error>> {
        // With buffer enabled via chain policy: 1x L1 cost should fail.
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST)
            .with_l1_data_fee_buffer(true);
        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        assert!(matches!(
            handler.pre_execution(&mut evm, &mut init_and_floor_gas),
            Err(EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee { .. }))
        ));

        // With buffer enabled: 2x L1 cost should pass.
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST + L1_DATA_COST)
            .with_l1_data_fee_buffer(true);
        let mut evm = ctx.build_scroll();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        assert!(handler.pre_execution(&mut evm, &mut init_and_floor_gas).is_ok());

        Ok(())
    }

    #[test]
    fn test_validate_l1_cost_no_buffer_by_default() -> Result<(), Box<dyn core::error::Error>> {
        // Without buffer: 1x L1_cost should pass
        let ctx = context()
            .with_scroll_spec(ScrollSpecId::CURIE)
            .with_funds(MIN_TRANSACTION_COST + L1_DATA_COST);
        let mut evm = ctx.build_scroll();
        let handler = ScrollHandler::<_, EVMError<_>, EthFrame<_>>::new();
        let mut init_and_floor_gas = handler.validate(&mut evm)?;
        assert!(handler.pre_execution(&mut evm, &mut init_and_floor_gas).is_ok());

        Ok(())
    }
}
