use std::borrow::Cow;
use std::{format, string::{String, ToString}};

use revm::context::{Cfg, ContextTr, JournalTr, LocalContextTr};
use revm::interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult};
use revm::precompile::{u64_to_address, Precompile, PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult};
use revm::primitives::{Address, Bytes};
use revm_primitives::{address, U256};
use crate::precompile::ScrollPrecompileProvider;
use crate::ScrollSpecId;

/// The Transfer precompile address.
pub const ADDRESS: Address = u64_to_address(0xff - 2);

/// The Transfer precompile id.
pub const ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("TRANSFER"));

/// The DOGE token contract address, which is the only allowed caller of the Transfer precompile.
pub const DOGE_TOKEN_CONTRACT_ADDRESS: Address = address!("0x000000000000000000000000000000000000d09e");

/// The Transfer precompile gas cost.
pub const GAS_COST: u64 = 9000;

/// The Transfer precompile enable specs.
pub const ENABLE_SPEC: ScrollSpecId = ScrollSpecId::GALILEO;

/// The dummy Transfer precompile
pub const DUMMY_PRECOMPILE: Precompile = Precompile::new(ID, ADDRESS, |_, _| unreachable!("dummy should not be called"));

impl ScrollPrecompileProvider {
    // copied from EthPrecompiles::run
    pub(crate) fn run_transfer<CTX: ContextTr<Cfg: Cfg<Spec = ScrollSpecId>>>(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<InterpreterResult>, String> {
        // -- PATCHED: pre check --
        let gas = Gas::new(inputs.gas_limit);

        // 1. can't transfer during static call
        if inputs.is_static {
            return Ok(Some(InterpreterResult {
                result: InstructionResult::StateChangeDuringStaticCall,
                gas,
                output: Bytes::new(),
            }));
        }

        // 2. only allow DOGE token contract to call this precompile
        if inputs.caller != DOGE_TOKEN_CONTRACT_ADDRESS {
            if context.journal().depth() == 1 {
                context
                    .local_mut()
                    .set_precompile_error_context("invalid caller for transfer precompile".to_string());
            }

            return Ok(Some(InterpreterResult {
                result: InstructionResult::PrecompileError,
                gas,
                output: Bytes::new(),
            }));
        }

        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas,
            output: Bytes::new(),
        };
        // -- END PATCH --

        let exec_result = {
            // -- PATCHED: extract journal --
            let (_, _, _, journal, _, local) = context.all_mut();
            // -- END PATCH --
            let r;
            let input_bytes = match &inputs.input {
                CallInput::SharedBuffer(range) => {
                    // -- PATCHED: fix borrow --
                    if let Some(slice) = local.shared_memory_buffer_slice(range.clone()) {
                    // -- END PATCH --
                        r = slice;
                        r.as_ref()
                    } else {
                        &[]
                    }
                }
                CallInput::Bytes(bytes) => bytes.0.iter().as_slice(),
            };
            // -- PATCHED: actual precompile execution --
            execute::<CTX>(journal, input_bytes, inputs.gas_limit)
            // -- END PATCH --
        };

        match exec_result {
            Ok(output) => {
                let underflow = result.gas.record_cost(output.gas_used);
                assert!(underflow, "Gas underflow is not possible");
                result.result = if output.reverted {
                    InstructionResult::Revert
                } else {
                    InstructionResult::Return
                };
                result.output = output.bytes;
            }
            Err(PrecompileError::Fatal(e)) => return Err(e),
            Err(e) => {
                result.result = if e.is_oog() {
                    InstructionResult::PrecompileOOG
                } else {
                    InstructionResult::PrecompileError
                };
                // If this is a top-level precompile call (depth == 1), persist the error message
                // into the local context so it can be returned as output in the final result.
                // Only do this for non-OOG errors (OOG is a distinct halt reason without output).
                if !e.is_oog() && context.journal().depth() == 1 {
                    context
                        .local_mut()
                        .set_precompile_error_context(e.to_string());
                }
            }
        }
        Ok(Some(result))
    }
}

// Parameters (abi-encoded): address from, address to, uint256 value
fn execute<CTX: ContextTr<Cfg: Cfg<Spec = ScrollSpecId>>>(
    journal: &mut <CTX as ContextTr>::Journal,
    input: &[u8],
    gas_limit: u64
) -> PrecompileResult {
    const CALL_DATA_LENGTH: usize = 32 + 32 + 32; // 3 parameters, each 32 bytes

    if gas_limit < GAS_COST {
        return Err(PrecompileError::OutOfGas);
    }

    if input.len() != CALL_DATA_LENGTH {
        return Err(PrecompileError::other("invalid transfer call input"));
    }

    let from = Address::from_slice(&input[12..32]);
    let to = Address::from_slice(&input[44..64]);
    let value = U256::from_be_slice(&input[64..96]);

    match journal.transfer(from, to, value) {
        Ok(None) => {},
        Ok(Some(e)) => return Err(PrecompileError::other(format!("transfer failed: {e:?}"))),
        Err(e) => return Err(PrecompileError::Fatal(e.to_string())),
    }

    Ok(PrecompileOutput {
        bytes: Bytes::new(),
        gas_used: GAS_COST,
        reverted: false,
    })
}
