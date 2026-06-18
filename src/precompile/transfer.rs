use std::{
    borrow::Cow,
    format,
    string::{String, ToString},
};

use crate::{precompile::ScrollPrecompileProvider, ScrollSpecId};
use revm::{
    context::{Cfg, ContextTr, JournalTr, LocalContextTr},
    handler::precompile_output_to_interpreter_result,
    interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult},
    precompile::{
        u64_to_address, Precompile, PrecompileError, PrecompileHalt, PrecompileId,
        PrecompileOutput, PrecompileResult,
    },
    primitives::{Address, Bytes},
};
use revm_primitives::U256;

/// The Transfer precompile address.
pub const ADDRESS: Address = u64_to_address(0xff - 2);

/// The Transfer precompile id.
pub const ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("TRANSFER"));

/// The Transfer precompile gas cost.
pub const GAS_COST: u64 = 9000;

/// The Transfer precompile enable specs.
pub const ENABLE_SPEC: ScrollSpecId = ScrollSpecId::GALDOGEOS;

/// The dummy Transfer precompile
pub const DUMMY_PRECOMPILE: Precompile =
    Precompile::new(ID, ADDRESS, |_, _, _| unreachable!("dummy should not be called"));

impl ScrollPrecompileProvider {
    // copied from EthPrecompiles::run
    pub(crate) fn run_transfer<CTX: ContextTr<Cfg: Cfg<Spec = ScrollSpecId>>>(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
        transfer_caller: Address,
    ) -> Result<Option<InterpreterResult>, String> {
        // -- PATCHED: pre check --
        let gas = Gas::new_with_regular_gas_and_reservoir(inputs.gas_limit, inputs.reservoir);

        // 1. can't transfer during static call
        if inputs.is_static {
            return Ok(Some(InterpreterResult {
                result: InstructionResult::StateChangeDuringStaticCall,
                gas,
                output: Bytes::new(),
            }));
        }

        // 2. only allow DOGE token contract to call this precompile
        if inputs.caller != transfer_caller {
            if context.journal().depth() == 1 {
                context.local_mut().set_precompile_error_context(
                    "invalid caller for transfer precompile".to_string(),
                );
            }

            let output = PrecompileOutput::halt(
                PrecompileHalt::other_static("invalid caller for transfer precompile"),
                inputs.reservoir,
            );
            return Ok(Some(precompile_output_to_interpreter_result(output, inputs.gas_limit)));
        }

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
            execute::<CTX>(journal, input_bytes, inputs.gas_limit, inputs.reservoir)
            // -- END PATCH --
        };

        match exec_result {
            Ok(output) => {
                if let Some(halt_reason) = output.halt_reason() {
                    if !halt_reason.is_oog() && context.journal().depth() == 1 {
                        context.local_mut().set_precompile_error_context(halt_reason.to_string());
                    }
                }
                Ok(Some(precompile_output_to_interpreter_result(output, inputs.gas_limit)))
            }
            Err(PrecompileError::Fatal(e)) => return Err(e),
            Err(e) => return Err(e.to_string()),
        }
    }
}

// Parameters (abi-encoded): address from, address to, uint256 value
fn execute<CTX: ContextTr<Cfg: Cfg<Spec = ScrollSpecId>>>(
    journal: &mut <CTX as ContextTr>::Journal,
    input: &[u8],
    gas_limit: u64,
    reservoir: u64,
) -> PrecompileResult {
    const CALL_DATA_LENGTH: usize = 32 + 32 + 32; // 3 parameters, each 32 bytes

    if gas_limit < GAS_COST {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
    }

    if input.len() != CALL_DATA_LENGTH {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("invalid transfer call input"),
            reservoir,
        ));
    }

    let from = Address::from_slice(&input[12..32]);
    let to = Address::from_slice(&input[44..64]);
    let value = U256::from_be_slice(&input[64..96]);

    // load account regardless of the value
    if let Some(e) =
        journal.transfer(from, to, value).map_err(|e| PrecompileError::Fatal(e.to_string()))?
    {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other(format!("transfer failed: {e:?}")),
            reservoir,
        ));
    }

    Ok(PrecompileOutput::new(GAS_COST, Bytes::new(), reservoir))
}
