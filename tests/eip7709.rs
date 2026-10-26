//! EIP-7709 activates at Tsuki and shares ordinary SLOAD access semantics.
use revm::{
    bytecode::{opcode::*, Bytecode},
    context::JournalTr,
    context_interface::context::ContextError,
    database::{Database, InMemoryDB},
    interpreter::{Gas, Host, InstructionResult, Interpreter},
    primitives::{address, bytes, Address, AddressMap, Bytes, HashSet, B256, U256},
    state::AccountInfo,
    DatabaseCommit, SystemCallEvm,
};
use revm_scroll::{
    builder::{DefaultScrollContext, ScrollBuilder, ScrollCfgExt, ScrollContext},
    instructions::make_scroll_instruction_table,
    ScrollSpecId,
};

const HISTORY: Address = address!("0000F90827F1C53a10cb7A02335B175320002935");
const CURRENT: u64 = 8192;
const NUMBER: u64 = CURRENT - 1;
const SLOT: U256 = U256::ZERO; // 8191 % 8191

fn context<DB: Database>(db: DB, spec: ScrollSpecId) -> ScrollContext<DB> {
    ScrollContext::scroll()
        .with_db(db)
        .modify_block_chained(|b| b.number = U256::from(CURRENT))
        .modify_cfg_chained(|c| c.set_scroll_spec(spec))
}

fn seeded(spec: ScrollSpecId) -> ScrollContext<InMemoryDB> {
    let mut db = InMemoryDB::default();
    db.insert_account_storage(HISTORY, SLOT, U256::from(42)).unwrap();
    context(db, spec)
}

fn run<DB: Database>(
    ctx: &mut ScrollContext<DB>,
    opcode: u8,
    argument: U256,
    gas: u64,
) -> (Interpreter, InstructionResult) {
    let table = make_scroll_instruction_table(ctx.cfg.spec);
    let mut interpreter =
        Interpreter::default().with_bytecode(Bytecode::new_legacy(Bytes::from(vec![opcode, STOP])));
    interpreter.gas = Gas::new(gas);
    interpreter.input.target_address = HISTORY;
    assert!(interpreter.stack.push(argument));
    let result = interpreter.run_plain(&table, ctx).instruction_result().unwrap();
    (interpreter, result)
}

fn read<DB: Database>(ctx: &mut ScrollContext<DB>, opcode: u8, arg: U256, gas: u64) -> U256 {
    let (mut interpreter, result) = run(ctx, opcode, arg, gas);
    assert_eq!(result, InstructionResult::Stop);
    assert_eq!(interpreter.gas.spent(), gas);
    interpreter.stack.pop().unwrap()
}

#[test]
fn tsuki_activation_preserves_earlier_forks() {
    for (spec, cold, warm) in [
        (ScrollSpecId::FEYNMAN, 20, 20),
        (ScrollSpecId::GALILEO, 20, 20),
        (ScrollSpecId::TSUKI, 2120, 120),
    ] {
        let mut ctx = seeded(spec);
        assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), cold), U256::from(42));
        assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), warm), U256::from(42));
    }
}

#[test]
fn blockhash_and_sload_share_slot_warmth_in_both_directions() {
    for blockhash_first in [false, true] {
        let mut ctx = seeded(ScrollSpecId::TSUKI);
        ctx.load_account_delegated(HISTORY).unwrap();
        let reads = if blockhash_first {
            [(BLOCKHASH, U256::from(NUMBER), 2120), (SLOAD, SLOT, 100)]
        } else {
            [(SLOAD, SLOT, 2100), (BLOCKHASH, U256::from(NUMBER), 120)]
        };
        for (opcode, argument, gas) in reads {
            assert_eq!(read(&mut ctx, opcode, argument, gas), U256::from(42));
        }
        // Warming one history slot does not warm its neighbours.
        read(&mut ctx, BLOCKHASH, U256::from(NUMBER - 1), 2120);
    }
}

#[test]
fn blockhash_preserves_account_warmth() {
    for warm_account in [false, true] {
        let mut ctx = seeded(ScrollSpecId::TSUKI);
        if warm_account {
            ctx.load_account_delegated(HISTORY).unwrap();
        }
        let child = ctx.journaled_state.checkpoint();
        read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
        let account_gas = if warm_account { 100 } else { 2600 };
        read(&mut ctx, BALANCE, HISTORY.into_word().into(), account_gas);
        ctx.journaled_state.checkpoint_revert(child);
        // Neither the BLOCKHASH nor the reverted BALANCE can leak account warmth.
        read(&mut ctx, BALANCE, HISTORY.into_word().into(), account_gas);
    }
}

#[test]
fn warm_blockhash_reads_do_not_grow_the_journal() {
    for warm_account in [false, true] {
        let mut ctx = context(CountingDb::default(), ScrollSpecId::TSUKI);
        if warm_account {
            ctx.load_account_delegated(HISTORY).unwrap();
        }
        let before = ctx.journaled_state.inner.journal.len();
        let child = ctx.journaled_state.checkpoint();
        read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
        // Only the slot becomes warm; preparing the history account adds no persistent entries.
        assert_eq!(ctx.journaled_state.inner.journal.len(), before + 1);
        for _ in 0..1000 {
            read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
        }
        assert_eq!(ctx.journaled_state.inner.journal.len(), before + 1);
        assert_eq!(ctx.journaled_state.depth(), 1);
        ctx.journaled_state.checkpoint_revert(child);
        assert_eq!(ctx.journaled_state.inner.journal.len(), before);
        assert_eq!(ctx.journaled_state.depth(), 0);
        let journal = &ctx.journaled_state.inner;
        assert_eq!(
            journal.state[&HISTORY].is_cold_transaction_id(journal.transaction_id),
            !warm_account
        );
        assert!(
            journal.state[&HISTORY].storage[&SLOT].is_cold_transaction_id(journal.transaction_id)
        );
        assert_eq!(ctx.journaled_state.database.basic_reads, 1);
        assert_eq!(ctx.journaled_state.database.storage_reads, 1);
    }
}

#[test]
fn cached_history_respects_transaction_boundaries_and_new_access_lists() {
    // 0: no access list; 1: account only; 2: account and slot.
    for previous_list in 0..=2 {
        for next_list in 0..=2 {
            let mut ctx = context(CountingDb::default(), ScrollSpecId::TSUKI);
            for (tx, list_kind) in [(0, previous_list), (1, next_list)] {
                if list_kind != 0 {
                    let mut list = AddressMap::default();
                    let slots = if list_kind == 2 {
                        HashSet::from_iter([SLOT])
                    } else {
                        HashSet::default()
                    };
                    list.insert(HISTORY, slots);
                    ctx.journaled_state.warm_access_list(list);
                }
                let gas = if list_kind == 2 { 120 } else { 2120 };
                let child = ctx.journaled_state.checkpoint();
                assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), gas), U256::from(42));
                read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
                if tx == 0 {
                    ctx.journaled_state.checkpoint_commit();
                    // End tx1 with a warm account so tx2 must handle cold-by-transaction-id too.
                    ctx.load_account_delegated(HISTORY).unwrap();
                    ctx.journaled_state.commit_tx();
                    let journal = &ctx.journaled_state.inner;
                    assert!(journal.state[&HISTORY].is_cold_transaction_id(journal.transaction_id));
                    assert!(journal.state[&HISTORY].storage[&SLOT]
                        .is_cold_transaction_id(journal.transaction_id));
                } else {
                    ctx.journaled_state.checkpoint_revert(child);
                    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), gas);
                    read(
                        &mut ctx,
                        BALANCE,
                        HISTORY.into_word().into(),
                        if list_kind == 0 { 2600 } else { 100 },
                    );
                }
            }
            assert_eq!(ctx.journaled_state.database.basic_reads, 1);
            assert_eq!(ctx.journaled_state.database.storage_reads, 1);
        }
    }
}

#[test]
fn committed_history_writes_remain_visible_in_the_next_transaction() {
    let mut ctx = seeded(ScrollSpecId::TSUKI);
    ctx.load_account_delegated(HISTORY).unwrap();
    ctx.journaled_state.sstore(HISTORY, SLOT, U256::from(99)).unwrap();
    ctx.journaled_state.commit_tx();
    assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120), U256::from(99));
    assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120), U256::from(99));
    let account = &ctx.journaled_state.inner.state[&HISTORY];
    assert_eq!(account.info, *account.original_info);
    assert!(!account.is_created_locally());
    assert!(!account.is_selfdestructed_locally());
}

#[test]
fn reverting_children_pay_cold_again_without_reloading_cached_values() {
    let mut ctx = context(CountingDb::default(), ScrollSpecId::TSUKI);
    let outer = ctx.journaled_state.checkpoint();
    for _ in 0..3 {
        let child = ctx.journaled_state.checkpoint();
        assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120), U256::from(42));
        assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120), U256::from(42));
        ctx.journaled_state.checkpoint_revert(child);
    }
    // A successful child is still undone by an outer revert.
    let _child = ctx.journaled_state.checkpoint();
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
    ctx.journaled_state.checkpoint_commit();
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
    ctx.journaled_state.checkpoint_revert(outer);
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
    let db = &ctx.journaled_state.database;
    assert_eq!(db.basic_reads, 1);
    assert_eq!(db.storage_reads, 1);
    assert_eq!(db.code_reads, 0);
}

#[test]
fn reverted_child_preserves_slot_warmed_by_parent() {
    let mut ctx = seeded(ScrollSpecId::TSUKI);
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
    let child = ctx.journaled_state.checkpoint();
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
    ctx.journaled_state.checkpoint_revert(child);
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
}

#[test]
fn access_list_warms_only_listed_slots_and_survives_revert() {
    for include_slot in [false, true] {
        let mut ctx = seeded(ScrollSpecId::TSUKI);
        let mut list = AddressMap::default();
        list.insert(
            HISTORY,
            if include_slot { HashSet::from_iter([SLOT]) } else { HashSet::default() },
        );
        ctx.journaled_state.warm_access_list(list);
        for _ in 0..2 {
            let child = ctx.journaled_state.checkpoint();
            let gas = if include_slot { 120 } else { 2120 };
            assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), gas), U256::from(42));
            ctx.journaled_state.checkpoint_revert(child);
        }
    }
}

#[test]
fn blockhash_observes_storage_writes_and_their_rollback() {
    let mut ctx = seeded(ScrollSpecId::TSUKI);
    assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120), U256::from(42));
    ctx.load_account_delegated(HISTORY).unwrap();
    let child = ctx.journaled_state.checkpoint();
    ctx.journaled_state.sstore(HISTORY, SLOT, U256::from(99)).unwrap();
    assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120), U256::from(99));
    ctx.journaled_state.checkpoint_revert(child);
    assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120), U256::from(42));
}

#[test]
fn history_window_and_ring_buffer_boundaries() {
    for current in [1u64, 256, 8191, 8192, 16383] {
        for age in [0, 1, 255, 256, 257] {
            let Some(number) = current.checked_sub(age) else { continue };
            let valid = (1..=256).contains(&age);
            let mut ctx = seeded(ScrollSpecId::TSUKI);
            ctx.block.number = U256::from(current);
            let key = U256::from(number % 8191);
            ctx.journaled_state
                .database
                .insert_account_storage(HISTORY, key, U256::from(7))
                .unwrap();
            assert_eq!(
                read(&mut ctx, BLOCKHASH, U256::from(number), if valid { 2120 } else { 20 }),
                if valid { U256::from(7) } else { U256::ZERO }
            );
            assert_eq!(ctx.journaled_state.inner.state.contains_key(&HISTORY), valid);
        }
    }
    for number in [U256::from(CURRENT + 1), U256::MAX] {
        let mut ctx = seeded(ScrollSpecId::TSUKI);
        assert_eq!(read(&mut ctx, BLOCKHASH, number, 20), U256::ZERO);
        assert!(ctx.journaled_state.inner.state.is_empty());
    }
}

#[test]
fn insufficient_gas_never_reads_or_warms_a_cold_slot() {
    for cached in [false, true] {
        for gas in [19, 20, 119, 120, 2119] {
            let mut ctx = context(CountingDb::default(), ScrollSpecId::TSUKI);
            if cached {
                let child = ctx.journaled_state.checkpoint();
                read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
                ctx.journaled_state.checkpoint_revert(child);
            }
            let (_, result) = run(&mut ctx, BLOCKHASH, U256::from(NUMBER), gas);
            assert_eq!(result, InstructionResult::OutOfGas);
            assert_eq!(ctx.journaled_state.database.storage_reads, usize::from(cached));
            // A failed read cannot make the next attempt warm, even before frame rollback.
            read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
            assert_eq!(ctx.journaled_state.database.storage_reads, 1);
        }
    }
    let mut ctx = seeded(ScrollSpecId::TSUKI);
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
    let (_, result) = run(&mut ctx, BLOCKHASH, U256::from(NUMBER), 119);
    assert_eq!(result, InstructionResult::OutOfGas);
    read(&mut ctx, BLOCKHASH, U256::from(NUMBER), 120);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Basic,
    Storage,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Failure {}
impl revm::database_interface::DBErrorMarker for Failure {}

#[derive(Default)]
struct CountingDb {
    fail: Option<Failure>,
    basic_reads: usize,
    storage_reads: usize,
    code_reads: usize,
}
impl Database for CountingDb {
    type Error = Failure;
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Failure> {
        assert_eq!(address, HISTORY);
        self.basic_reads += 1;
        if self.fail == Some(Failure::Basic) {
            return Err(Failure::Basic);
        }
        Ok(Some(AccountInfo {
            nonce: 1,
            code_hash: B256::repeat_byte(1),
            code: None,
            ..Default::default()
        }))
    }
    fn code_by_hash(&mut self, _: B256) -> Result<Bytecode, Failure> {
        self.code_reads += 1;
        Ok(Bytecode::new_legacy(Bytes::from(vec![STOP])))
    }
    fn storage(&mut self, address: Address, _: U256) -> Result<U256, Failure> {
        assert_eq!(address, HISTORY);
        self.storage_reads += 1;
        if self.fail == Some(Failure::Storage) {
            return Err(Failure::Storage);
        }
        Ok(U256::from(42))
    }
    fn block_hash(&mut self, _: u64) -> Result<B256, Failure> {
        panic!("unexpected ancestor lookup")
    }
}

#[test]
fn database_errors_keep_their_original_type() {
    for failure in [Failure::Basic, Failure::Storage] {
        let mut ctx =
            context(CountingDb { fail: Some(failure), ..Default::default() }, ScrollSpecId::TSUKI);
        let (_, result) = run(&mut ctx, BLOCKHASH, U256::from(NUMBER), 2120);
        assert_eq!(result, InstructionResult::FatalExternalError);
        assert!(matches!(ctx.error, Err(ContextError::Db(error)) if error == failure));
        assert_eq!(ctx.journaled_state.depth(), 0);
        assert!(ctx.journaled_state.inner.journal.is_empty());
    }
}

#[test]
fn history_bytecode_is_only_loaded_before_tsuki() {
    for (spec, gas, code_reads) in [(ScrollSpecId::GALILEO, 20, 1), (ScrollSpecId::TSUKI, 2120, 0)]
    {
        let mut ctx = context(CountingDb::default(), spec);
        assert_eq!(read(&mut ctx, BLOCKHASH, U256::from(NUMBER), gas), U256::from(42));
        assert_eq!(ctx.journaled_state.database.code_reads, code_reads);
    }
}

#[test]
fn system_contract_updates_are_visible_after_ring_buffer_wrap() {
    let mut ctx = seeded(ScrollSpecId::TSUKI);
    // Canonical runtime bytecode from https://eips.ethereum.org/EIPS/eip-2935.
    let code = Bytecode::new_legacy(bytes!(
        "3373fffffffffffffffffffffffffffffffffffffffe14604657602036036042575f35600143038111604257611fff81430311604257611fff9006545f5260205ff35b5f5ffd5b5f35611fff60014303065500"
    ));
    ctx.journaled_state
        .database
        .insert_account_info(HISTORY, AccountInfo::new(U256::ZERO, 1, code.hash_slow(), code));
    let mut evm = ctx.build_scroll();
    for (current, hash) in [(CURRENT, 77u64), (CURRENT + 8191, 99)] {
        evm.0.ctx.block.number = U256::from(current);
        let hash = U256::from(hash);
        let result =
            evm.system_call(HISTORY, Bytes::copy_from_slice(&hash.to_be_bytes::<32>())).unwrap();
        assert!(result.result.is_success());
        evm.0.ctx.journaled_state.database.commit(result.state);
        assert_eq!(read(&mut evm.0.ctx, BLOCKHASH, U256::from(current - 1), 2120), hash);
    }
}

#[test]
fn real_staticcall_frames_charge_cold_again_only_after_revert() {
    const PARENT: Address = address!("1000000000000000000000000000000000000001");
    const CHILD: Address = address!("1000000000000000000000000000000000000002");
    for spec in [ScrollSpecId::GALILEO, ScrollSpecId::TSUKI] {
        for revert in [false, true] {
            let mut ctx = seeded(spec);
            // Measure one BLOCKHASH with GAS and return the measured cost, including on REVERT.
            let child = vec![
                GAS,
                PUSH2,
                0x1f,
                0xff,
                BLOCKHASH,
                POP,
                GAS,
                SWAP1,
                SUB,
                PUSH0,
                MSTORE,
                PUSH1,
                32,
                PUSH0,
                if revert { REVERT } else { RETURN },
            ];
            let mut parent = Vec::new();
            for offset in [0, 32] {
                parent.extend([PUSH1, 32, PUSH1, offset, PUSH0, PUSH0, PUSH20]);
                parent.extend(CHILD.as_slice());
                parent.extend([PUSH3, 1, 0x86, 0xa0, STATICCALL, POP]);
            }
            parent.extend([PUSH1, 64, PUSH0, RETURN]);
            for (address, code) in [(PARENT, parent), (CHILD, child)] {
                let code = Bytecode::new_legacy(Bytes::from(code));
                ctx.journaled_state.database.insert_account_info(
                    address,
                    AccountInfo::new(U256::ZERO, 1, code.hash_slow(), code),
                );
            }
            let result = ctx.build_scroll().system_call(PARENT, Bytes::new()).unwrap().result;
            assert!(result.is_success());
            let output = result.output().unwrap();
            assert_eq!(output.len(), 64);
            let costs: [u64; 2] = if spec == ScrollSpecId::TSUKI {
                [2120, if revert { 2120 } else { 120 }]
            } else {
                [20, 20]
            };
            for (word, gas) in output.chunks_exact(32).zip(costs) {
                // PUSH2 + POP + the second GAS add 7 gas to each measurement.
                assert_eq!(U256::from_be_slice(word), U256::from(gas + 7));
            }
        }
    }
}
