use crate::{exec::ScrollContextTr, ScrollSpecId};
use core::cmp::max;
use revm::{
    bytecode::opcode,
    context::Cfg,
    handler::instructions::InstructionProvider,
    interpreter::{
        as_u64_saturated, as_usize_or_fail, gas, instruction_table,
        instructions::gas_table_spec,
        interpreter_types::{InputsTr, MemoryTr, RuntimeFlag, StackTr},
        popn, popn_top, push, require_non_staticcall, GasTable, Host, Instruction,
        InstructionContext, InstructionExecResult, InstructionResult, InstructionTable,
        InterpreterTypes,
    },
    primitives::{address, hardfork::SpecId, keccak256, Address, BLOCK_HASH_HISTORY, U256},
};
use std::rc::Rc;

const HISTORY_STORAGE_ADDRESS: Address = address!("0x0000F90827F1C53a10cb7A02335B175320002935");
const HISTORY_SERVE_WINDOW: u64 = 8191;
const DIFFICULTY: U256 = U256::ZERO;

/// Holds the EVM instruction table for Scroll.
pub struct ScrollInstructions<WIRE: InterpreterTypes, HOST> {
    pub instruction_table: Rc<InstructionTable<WIRE, HOST>>,
    pub gas_table: Rc<GasTable>,
}

impl<IT, CTX> InstructionProvider for ScrollInstructions<IT, CTX>
where
    IT: InterpreterTypes,
    CTX: Host,
{
    type InterpreterTypes = IT;
    type Context = CTX;

    fn instruction_table(&self) -> &InstructionTable<Self::InterpreterTypes, Self::Context> {
        &self.instruction_table
    }

    fn gas_table(&self) -> &GasTable {
        &self.gas_table
    }
}

impl<WIRE, HOST> Clone for ScrollInstructions<WIRE, HOST>
where
    WIRE: InterpreterTypes,
{
    fn clone(&self) -> Self {
        Self {
            instruction_table: self.instruction_table.clone(),
            gas_table: self.gas_table.clone(),
        }
    }
}

impl<WIRE, HOST> ScrollInstructions<WIRE, HOST>
where
    WIRE: InterpreterTypes,
    HOST: ScrollContextTr,
{
    pub fn new_mainnet() -> Self {
        Self::new(make_scroll_instruction_table::<WIRE, HOST>(), make_scroll_gas_table())
    }

    pub fn new(instruction_table: InstructionTable<WIRE, HOST>, gas_table: GasTable) -> Self {
        Self { instruction_table: Rc::new(instruction_table), gas_table: Rc::new(gas_table) }
    }
}

/// Creates a table of instructions for the Scroll hardfork.
///
/// The following instructions are overridden:
/// - `BLOCKHASH`
/// - `BASEFEE`
/// - `TSTORE`
/// - `TLOAD`
/// - `SELFDESTRUCT`
/// - `MCOPY`
/// - `DIFFICULTY`
/// - `CLZ`
pub fn make_scroll_instruction_table<WIRE: InterpreterTypes, HOST: ScrollContextTr>(
) -> InstructionTable<WIRE, HOST> {
    let mut table = instruction_table::<WIRE, HOST>();

    // override the instructions
    table[opcode::BLOCKHASH as usize] = Instruction::new(blockhash::<WIRE, HOST>);
    table[opcode::BASEFEE as usize] = Instruction::new(basefee::<WIRE, HOST>);
    table[opcode::TSTORE as usize] = Instruction::new(tstore::<WIRE, HOST>);
    table[opcode::TLOAD as usize] = Instruction::new(tload::<WIRE, HOST>);
    table[opcode::SELFDESTRUCT as usize] = Instruction::new(selfdestruct::<WIRE, HOST>);
    table[opcode::MCOPY as usize] = Instruction::new(mcopy::<WIRE, HOST>);
    table[opcode::DIFFICULTY as usize] = Instruction::new(difficulty::<WIRE, HOST>);
    table[opcode::CLZ as usize] = Instruction::new(clz::<WIRE, HOST>);

    table
}

/// Creates a static gas table for Scroll instructions.
pub fn make_scroll_gas_table() -> GasTable {
    let mut table = gas_table_spec(SpecId::SHANGHAI);
    
    table[opcode::SELFDESTRUCT as usize] = 0;

    table
}

// SHANGHAI OPCODE IMPLEMENTATIONS
// ================================================================================================

/// Computes the blockhash for the requested block number.
///
/// If the requested block number is the current block number, a future block number or a block
/// number older than `BLOCK_HASH_HISTORY` we return 0.
/// Gas is accounted in the interpreter <https://github.com/bluealloy/revm/blob/fd52a1fb531f4627ea7e69780aab56536533269d/crates/interpreter/src/interpreter.rs#L278>
fn blockhash<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;

    popn_top!([], number, interpreter);

    let requested_number = *number;
    let block_number = host.block_number();

    // compute the diff between the current block number and the requested block number
    let Some(diff) = block_number.checked_sub(requested_number) else {
        *number = U256::ZERO;
        return Ok(());
    };

    let diff = as_u64_saturated!(diff);
    *number = match diff {
        // blockhash requested for current or future block - return 0
        0 => U256::ZERO,
        // blockhash requested for block older than BLOCK_HASH_HISTORY - return 0
        x if x > BLOCK_HASH_HISTORY => U256::ZERO,
        // blockhash requested for block in the history (pre-Feynman)
        // blockhash is computed as the keccak256 hash of the chain id and the block number
        _ if !host.cfg().spec().is_enabled_in(ScrollSpecId::FEYNMAN) => {
            let chain_id = as_u64_saturated!(host.chain_id());
            compute_block_hash(chain_id, as_u64_saturated!(requested_number))
        }
        // blockhash requested for block in the history (post-Feynman)
        // blockhash is loaded from the EIP-2935 history storage system contract storage.
        _ => {
            // sload assumes that the account is present in the journal
            if host.load_account_delegated(HISTORY_STORAGE_ADDRESS).is_none() {
                return Err(InstructionResult::FatalExternalError);
            };

            // index in system contract ring buffer storage is block_number % HISTORY_SERVE_WINDOW
            let requested_block_number_u64 = as_u64_saturated!(requested_number);
            let index = requested_block_number_u64.wrapping_rem(HISTORY_SERVE_WINDOW);

            let Some(value) = host.sload(HISTORY_STORAGE_ADDRESS, U256::from(index)) else {
                return Err(InstructionResult::FatalExternalError);
            };

            value.data
        }
    };
    Ok(())
}

/// Implements the SELFDESTRUCT instruction.
///
/// Halt execution and register account for later deletion.
fn selfdestruct<WIRE: InterpreterTypes, H: Host>(
    _context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    Err(InstructionResult::NotActivated)
}

// CURIE OPCODE IMPLEMENTATIONS
// ================================================================================================

/// EIP-3198: BASEFEE opcode
/// Gas is accounted in the interpreter <https://github.com/bluealloy/revm/blob/fd52a1fb531f4627ea7e69780aab56536533269d/crates/interpreter/src/interpreter.rs#L278>
fn basefee<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;
    if !host.cfg().spec().is_enabled_in(ScrollSpecId::CURIE) {
        return Err(InstructionResult::NotActivated);
    }

    push!(interpreter, U256::from(host.basefee()));
    Ok(())
}

/// Store transient storage tied to the account.
///
/// If values is different add entry to the journal
/// so that old state can be reverted if that action is needed.
///
/// EIP-1153: Transient storage opcodes
/// Gas is accounted in the interpreter <https://github.com/bluealloy/revm/blob/fd52a1fb531f4627ea7e69780aab56536533269d/crates/interpreter/src/interpreter.rs#L278>
fn tstore<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;
    if !host.cfg().spec().is_enabled_in(ScrollSpecId::CURIE) {
        return Err(InstructionResult::NotActivated);
    }

    require_non_staticcall!(interpreter);

    popn!([index, value], interpreter);

    host.tstore(interpreter.input.target_address(), index, value);
    Ok(())
}

/// Read transient storage tied to the account.
///
/// EIP-1153: Transient storage opcodes
/// Gas is accounted in the interpreter <https://github.com/bluealloy/revm/blob/fd52a1fb531f4627ea7e69780aab56536533269d/crates/interpreter/src/interpreter.rs#L278>
fn tload<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;
    if !host.cfg().spec().is_enabled_in(ScrollSpecId::CURIE) {
        return Err(InstructionResult::NotActivated);
    }

    popn_top!([], index, interpreter);

    *index = host.tload(interpreter.input.target_address(), *index);
    Ok(())
}

/// Implements the MCOPY instruction.
///
/// EIP-5656: Memory copying instruction that copies memory from one location to another.
fn mcopy<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;
    if !host.cfg().spec().is_enabled_in(ScrollSpecId::CURIE) {
        return Err(InstructionResult::NotActivated);
    }

    popn!([dst, src, len], interpreter);

    // into usize or fail
    let len = as_usize_or_fail!(interpreter, len);
    // deduce gas
    gas!(interpreter, host.gas_params().mcopy_cost(len));
    if len == 0 {
        return Ok(());
    }

    let dst = as_usize_or_fail!(interpreter, dst);
    let src = as_usize_or_fail!(interpreter, src);
    // resize memory
    interpreter.resize_memory(host.gas_params(), max(dst, src), len)?;
    // copy memory in place
    interpreter.memory.copy(dst, src, len);
    Ok(())
}

/// Implements the DIFFICULTY instruction.
///
/// Pushes the block difficulty(default to 0) onto the stack.
/// Gas is accounted in the interpreter <https://github.com/bluealloy/revm/blob/fd52a1fb531f4627ea7e69780aab56536533269d/crates/interpreter/src/interpreter.rs#L278>
pub fn difficulty<WIRE: InterpreterTypes, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    push!(context.interpreter, DIFFICULTY);
    Ok(())
}

/// Implements the CLZ instruction
///
/// EIP-7939 count leading zeros.
fn clz<WIRE: InterpreterTypes, H: ScrollContextTr>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    let host = context.host;
    let interpreter = context.interpreter;
    if !host.cfg().spec().is_enabled_in(ScrollSpecId::GALILEO) {
        return Err(InstructionResult::NotActivated);
    }

    popn_top!([], op1, interpreter);

    let leading_zeros = op1.leading_zeros();
    *op1 = U256::from(leading_zeros);
    Ok(())
}

// HELPER FUNCTIONS
// ================================================================================================

/// Helper function to compute the block hash.
///
/// The block hash is computed as the keccak256 hash of the chain id and the block number.
fn compute_block_hash(chain_id: u64, block_number: u64) -> U256 {
    let mut input = [0u8; 16];
    input[..8].copy_from_slice(&chain_id.to_be_bytes());
    input[8..].copy_from_slice(&block_number.to_be_bytes());
    U256::from_be_bytes(keccak256(input).into())
}

#[cfg(test)]
mod tests {
    use super::{clz, compute_block_hash, make_scroll_gas_table, make_scroll_instruction_table};
    use crate::{
        builder::{DefaultScrollContext, ScrollContext},
        instructions::HISTORY_STORAGE_ADDRESS,
        ScrollSpecId::*,
    };

    use revm::{
        bytecode::{opcode::*, Bytecode},
        database::{EmptyDB, InMemoryDB},
        interpreter::{InstructionContext, Interpreter},
        primitives::{Bytes, U256},
        DatabaseRef,
    };
    use rstest::rstest;

    #[test]
    fn test_blockhash_before_feynman() {
        let (chain_id, current_block, target_block, spec) = (123, U256::from(1024), 1000, EUCLID);

        let db = EmptyDB::new();
        let mut context = ScrollContext::scroll().with_db(InMemoryDB::new(db));
        context.modify_block(|block| block.number = current_block);
        context.modify_cfg(|cfg| cfg.chain_id = chain_id);
        context.modify_cfg(|cfg| cfg.spec = spec);

        let instructions = make_scroll_instruction_table();
        let gas_table = make_scroll_gas_table();

        let bytecode = Bytecode::new_legacy(Bytes::from(&[BLOCKHASH, STOP]));
        let mut interpreter = Interpreter::default().with_bytecode(bytecode);
        let _ = interpreter.stack.push(U256::from(target_block));
        interpreter.run_plain(&instructions, &gas_table, &mut context);

        let expected = compute_block_hash(chain_id, target_block);
        let actual = interpreter.stack.pop().expect("stack is not empty");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_blockhash_after_feynman() {
        let (chain_id, current_block, target_block, spec) = (123, U256::from(1024), 1000, FEYNMAN);

        let db = EmptyDB::new();
        let mut context = ScrollContext::scroll().with_db(InMemoryDB::new(db));
        context.modify_block(|block| block.number = current_block);
        context.modify_cfg(|cfg| cfg.chain_id = chain_id);
        context.modify_cfg(|cfg| cfg.spec = spec);

        // updating the history storage system contract is not part of revm,
        // in this test we simply write the block hash to the contract storage.
        let expected_block_hash = db.block_hash_ref(target_block).expect("db contains block hash");
        context.modify_db(|db| {
            db.insert_account_storage(
                HISTORY_STORAGE_ADDRESS,
                U256::from(target_block),
                expected_block_hash.into(),
            )
            .expect("insert account should succeed")
        });

        let instructions = make_scroll_instruction_table();
        let gas_table = make_scroll_gas_table();

        let bytecode = Bytecode::new_legacy(Bytes::from(&[BLOCKHASH, STOP]));
        let mut interpreter = Interpreter::default().with_bytecode(bytecode);
        let _ = interpreter.stack.push(U256::from(target_block));
        interpreter.run_plain(&instructions, &gas_table, &mut context);

        let expected: U256 = expected_block_hash.into();
        let actual = interpreter.stack.pop().expect("stack is not empty");
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case(BLOCKHASH, 20)]
    #[case(BASEFEE, 2)]
    #[case(TSTORE, 100)]
    #[case(TLOAD, 100)]
    #[case(MCOPY, 9)]
    #[case(SELFDESTRUCT, 0)]
    #[case(DIFFICULTY, 2)]
    fn test_gas_used(#[case] opcode: u8, #[case] expected_gas_used: u64) {
        let (chain_id, current_block, spec) = (123, U256::from(1024), FEYNMAN);

        let db = EmptyDB::new();
        let mut context = ScrollContext::scroll().with_db(InMemoryDB::new(db));
        context.modify_block(|block| block.number = current_block);
        context.modify_cfg(|cfg| cfg.chain_id = chain_id);
        context.modify_cfg(|cfg| cfg.spec = spec);

        let instructions = make_scroll_instruction_table();
        let gas_table = make_scroll_gas_table();

        let bytecode = Bytecode::new_legacy(Bytes::from([opcode, STOP].to_vec()));
        let mut interpreter = Interpreter::default().with_bytecode(bytecode);
        let _ = interpreter.stack.push(U256::from(1));
        let _ = interpreter.stack.push(U256::from(0));
        let _ = interpreter.stack.push(U256::from(0));
        interpreter.run_plain(&instructions, &gas_table, &mut context);

        let actual_gas_used = interpreter.gas.used();
        assert_eq!(actual_gas_used, expected_gas_used);
    }

    #[test]
    fn test_clz() {
        use revm::primitives::uint;

        let spec = GALILEO;
        let db = EmptyDB::new();
        let mut scroll_context = ScrollContext::scroll().with_db(InMemoryDB::new(db));
        scroll_context.modify_cfg(|cfg| cfg.spec = spec);

        let mut interpreter = Interpreter::default();

        struct TestCase {
            value: U256,
            expected: U256,
        }

        uint! {
            let test_cases = [
                TestCase { value: 0x0_U256, expected: 256_U256 },
                TestCase { value: 0x1_U256, expected: 255_U256 },
                TestCase { value: 0x2_U256, expected: 254_U256 },
                TestCase { value: 0x3_U256, expected: 254_U256 },
                TestCase { value: 0x4_U256, expected: 253_U256 },
                TestCase { value: 0x7_U256, expected: 253_U256 },
                TestCase { value: 0x8_U256, expected: 252_U256 },
                TestCase { value: 0xff_U256, expected: 248_U256 },
                TestCase { value: 0x100_U256, expected: 247_U256 },
                TestCase { value: 0xffff_U256, expected: 240_U256 },
                TestCase {
                    value: 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256, // U256::MAX
                    expected: 0_U256,
                },
                TestCase {
                    value: 0x8000000000000000000000000000000000000000000000000000000000000000_U256, // 1 << 255
                    expected: 0_U256,
                },
                TestCase { // Smallest value with 1 leading zero
                    value: 0x4000000000000000000000000000000000000000000000000000000000000000_U256, // 1 << 254
                    expected: 1_U256,
                },
                TestCase { // Value just below 1 << 255
                    value: 0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_U256,
                    expected: 1_U256,
                },
            ];
        }

        for test in test_cases {
            assert!(interpreter.stack.push(test.value));
            let context =
                InstructionContext { host: &mut scroll_context, interpreter: &mut interpreter };
            clz(context).unwrap();
            let res = interpreter.stack.pop().unwrap();
            assert_eq!(
                res, test.expected,
                "CLZ for value {:#x} failed. Expected: {}, Got: {}",
                test.value, test.expected, res
            );
        }
    }
}
