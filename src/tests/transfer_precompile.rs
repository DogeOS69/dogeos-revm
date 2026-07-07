use crate::{
    builder::{DefaultScrollContext, ScrollContext},
    precompile::{
        self,
        transfer::{
            ADDRESS as TRANSFER_ADDRESS, GAS_COST as TRANSFER_GAS_COST,
            NATIVE_DOGE_TOKEN_ADDRESS as ALLOWED_CALLER,
        },
        ScrollPrecompileProvider,
    },
    ScrollSpecId,
};
use alloy_evm::{
    precompiles::{Precompile, PrecompileInput},
    EvmInternals,
};
use revm::{
    context::{ContextTr, JournalTr},
    database::{DbAccount, InMemoryDB},
    database_interface::DBErrorMarker,
    handler::PrecompileProvider,
    interpreter::{CallInput, CallInputs, CallScheme, CallValue, InstructionResult},
    precompile::{u64_to_address, PrecompileError, PrecompileResult},
    state::{AccountInfo, Bytecode},
    Context, Database,
};
use revm_primitives::{address, Address, Bytes, StorageKey, StorageValue, B256, U256};
use std::{boxed::Box, fmt, vec, vec::Vec};

const DISALLOWED_CALLER: Address = address!("0x0000000000000000000000000000000000002000");
const FROM: Address = address!("0x0000000000000000000000000000000000003000");
const TO: Address = address!("0x0000000000000000000000000000000000004000");
const OTHER: Address = address!("0x0000000000000000000000000000000000005000");

#[derive(Clone, Copy)]
struct TransferCall<'a> {
    caller: Address,
    target_address: Address,
    bytecode_address: Address,
    value: U256,
    input: &'a [u8],
    gas: u64,
    is_static: bool,
}

impl<'a> TransferCall<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            caller: ALLOWED_CALLER,
            target_address: TRANSFER_ADDRESS,
            bytecode_address: TRANSFER_ADDRESS,
            value: U256::ZERO,
            input,
            gas: TRANSFER_GAS_COST,
            is_static: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FailingDb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FailingDbError;

impl fmt::Display for FailingDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failing test database")
    }
}

impl core::error::Error for FailingDbError {}
impl DBErrorMarker for FailingDbError {}

impl Database for FailingDb {
    type Error = FailingDbError;

    fn basic(&mut self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Err(FailingDbError)
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        Err(FailingDbError)
    }

    fn storage(
        &mut self,
        _address: Address,
        _index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        Err(FailingDbError)
    }

    fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
        Err(FailingDbError)
    }
}

fn transfer_input(from: Address, to: Address, value: U256) -> Vec<u8> {
    let mut input = vec![0; 96];
    input[12..32].copy_from_slice(from.as_slice());
    input[44..64].copy_from_slice(to.as_slice());
    input[64..96].copy_from_slice(&value.to_be_bytes::<32>());
    input
}

fn context(from_balance: U256, to_balance: U256) -> ScrollContext<InMemoryDB> {
    context_with_balances(&[(FROM, from_balance), (TO, to_balance)])
}

fn context_with_balances(balances: &[(Address, U256)]) -> ScrollContext<InMemoryDB> {
    Context::scroll()
        .with_db(InMemoryDB::default())
        .modify_cfg_chained(|cfg| cfg.spec = ScrollSpecId::TSUKI)
        .modify_db_chained(|db| {
            for (address, balance) in balances {
                db.cache.accounts.insert(
                    *address,
                    DbAccount {
                        info: AccountInfo { balance: *balance, ..Default::default() },
                        ..Default::default()
                    },
                );
            }
        })
}

fn failing_context() -> ScrollContext<FailingDb> {
    Context::scroll().with_db(FailingDb).modify_cfg_chained(|cfg| cfg.spec = ScrollSpecId::TSUKI)
}

fn call_transfer_precompile<DB>(
    ctx: &mut ScrollContext<DB>,
    call: TransferCall<'_>,
) -> PrecompileResult
where
    DB: Database + fmt::Debug,
{
    let precompiles = precompile::tsuki();
    let precompile =
        precompiles.get(&TRANSFER_ADDRESS).expect("transfer precompile exists in TSUKI");

    precompile.call(PrecompileInput {
        data: call.input,
        gas: call.gas,
        caller: call.caller,
        value: call.value,
        target_address: call.target_address,
        is_static: call.is_static,
        bytecode_address: call.bytecode_address,
        internals: EvmInternals::from_context(ctx),
    })
}

fn expect_other_contains(result: PrecompileResult, expected: &str) {
    let err = result.expect_err("precompile call should fail");
    assert!(
        matches!(&err, PrecompileError::Other(msg) if msg.contains(expected)),
        "unexpected precompile error: {err:?}"
    );
}

fn balance(
    ctx: &mut ScrollContext<InMemoryDB>,
    address: Address,
) -> Result<U256, Box<dyn core::error::Error>> {
    Ok(ctx.journal_mut().load_account(address)?.data.info.balance)
}

fn direct_call_error() -> &'static str {
    "must be called directly"
}

#[test]
fn allowed_caller_succeeds() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(100), U256::from(7));
    let input = transfer_input(FROM, TO, U256::from(41));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(59));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(48));
    Ok(())
}

#[test]
fn disallowed_caller_rejected() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall { caller: DISALLOWED_CALLER, ..TransferCall::new(&input) },
        ),
        "allowed caller",
    );
}

#[test]
fn delegatecall_rejected_even_with_allowed_caller() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall { target_address: OTHER, ..TransferCall::new(&input) },
        ),
        direct_call_error(),
    );
}

#[test]
fn callcode_rejected() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall {
                caller: DISALLOWED_CALLER,
                target_address: OTHER,
                bytecode_address: TRANSFER_ADDRESS,
                ..TransferCall::new(&input)
            },
        ),
        direct_call_error(),
    );
}

#[test]
fn static_call_rejected() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall { is_static: true, ..TransferCall::new(&input) },
        ),
        "static context",
    );
}

#[test]
fn static_beats_caller() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall {
                caller: DISALLOWED_CALLER,
                is_static: true,
                ..TransferCall::new(&input)
            },
        ),
        "static context",
    );
}

#[test]
fn delegatecall_beats_caller() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall {
                caller: DISALLOWED_CALLER,
                target_address: OTHER,
                ..TransferCall::new(&input)
            },
        ),
        direct_call_error(),
    );
}

#[test]
fn caller_beats_gas() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall {
                caller: DISALLOWED_CALLER,
                gas: TRANSFER_GAS_COST - 1,
                ..TransferCall::new(&input)
            },
        ),
        "allowed caller",
    );
}

#[test]
fn caller_beats_length() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = vec![0; 95];

    expect_other_contains(
        call_transfer_precompile(
            &mut ctx,
            TransferCall { caller: DISALLOWED_CALLER, ..TransferCall::new(&input) },
        ),
        "allowed caller",
    );
}

#[test]
fn insufficient_gas_oog() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    let err = call_transfer_precompile(
        &mut ctx,
        TransferCall { gas: TRANSFER_GAS_COST - 1, ..TransferCall::new(&input) },
    )
    .expect_err("insufficient gas should fail");

    assert_eq!(err, PrecompileError::OutOfGas);
}

#[test]
fn exact_gas_succeeds() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    let output = call_transfer_precompile(
        &mut ctx,
        TransferCall { gas: TRANSFER_GAS_COST, ..TransferCall::new(&input) },
    )?;

    assert!(!output.reverted);
    assert_eq!(output.gas_used, TRANSFER_GAS_COST);
    Ok(())
}

#[test]
fn gas_used_on_success_is_fixed() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(1));

    let output = call_transfer_precompile(
        &mut ctx,
        TransferCall { gas: TRANSFER_GAS_COST * 3, ..TransferCall::new(&input) },
    )?;

    assert_eq!(output.gas_used, TRANSFER_GAS_COST);
    Ok(())
}

#[test]
fn gas_used_on_revert_is_fixed() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(1), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(2));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(output.reverted);
    assert_eq!(output.gas_used, TRANSFER_GAS_COST);
    Ok(())
}

#[test]
fn wrong_length_short() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = vec![0; 95];

    expect_other_contains(
        call_transfer_precompile(&mut ctx, TransferCall::new(&input)),
        "96 bytes",
    );
}

#[test]
fn wrong_length_long() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = vec![0; 97];

    expect_other_contains(
        call_transfer_precompile(&mut ctx, TransferCall::new(&input)),
        "96 bytes",
    );
}

#[test]
fn empty_calldata() {
    let mut ctx = context(U256::from(100), U256::ZERO);
    let input = vec![];

    expect_other_contains(
        call_transfer_precompile(&mut ctx, TransferCall::new(&input)),
        "96 bytes",
    );
}

#[test]
fn exact_96_bytes_parses() -> Result<(), Box<dyn core::error::Error>> {
    let value = U256::from(13);
    let input = transfer_input(FROM, TO, value);
    let mut ctx = context(U256::from(20), U256::from(3));

    assert_eq!(&input[12..32], FROM.as_slice());
    assert_eq!(&input[44..64], TO.as_slice());
    assert_eq!(&input[64..96], value.to_be_bytes::<32>().as_slice());

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(7));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(16));
    Ok(())
}

#[test]
fn nonzero_from_padding_is_ignored_by_current_parser() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::ZERO);
    let mut input = transfer_input(FROM, TO, U256::from(1));
    input[0] = 0xff;

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(9));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(1));
    Ok(())
}

#[test]
fn nonzero_to_padding_is_ignored_by_current_parser() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::ZERO);
    let mut input = transfer_input(FROM, TO, U256::from(1));
    input[32] = 0xff;

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(9));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(1));
    Ok(())
}

#[test]
fn successful_transfer_moves_balance() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(100), U256::from(7));
    let input = transfer_input(FROM, TO, U256::from(41));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(59));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(48));
    Ok(())
}

#[test]
fn insufficient_funds_reverts() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(40), U256::from(7));
    let input = transfer_input(FROM, TO, U256::from(41));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(output.reverted);
    assert_eq!(output.gas_used, TRANSFER_GAS_COST);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(40));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(7));
    Ok(())
}

#[test]
fn overflow_payment_returns_reverted_result() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(1), U256::MAX);
    let input = transfer_input(FROM, TO, U256::from(1));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(output.reverted);
    assert_eq!(output.gas_used, TRANSFER_GAS_COST);
    Ok(())
}

#[test]
fn zero_value_transfer() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::from(3));
    let input = transfer_input(FROM, TO, U256::ZERO);

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(10));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(3));
    Ok(())
}

#[test]
fn self_transfer_from_equals_to() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context_with_balances(&[(FROM, U256::from(10))]);
    let input = transfer_input(FROM, FROM, U256::from(7));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(10));
    Ok(())
}

#[test]
fn full_balance_transfer() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::from(3));
    let input = transfer_input(FROM, TO, U256::from(10));

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::ZERO);
    assert_eq!(balance(&mut ctx, TO)?, U256::from(13));
    Ok(())
}

#[test]
fn transfer_is_journaled_revertible() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::from(3));
    let input = transfer_input(FROM, TO, U256::from(4));
    let checkpoint = ctx.journal_mut().checkpoint();

    let output = call_transfer_precompile(&mut ctx, TransferCall::new(&input))?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(6));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(7));

    ctx.journal_mut().checkpoint_revert(checkpoint);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(10));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(3));
    Ok(())
}

#[test]
fn db_error_is_fatal() {
    let mut ctx = failing_context();
    let input = transfer_input(FROM, TO, U256::from(1));

    let err = call_transfer_precompile(&mut ctx, TransferCall::new(&input))
        .expect_err("db error should be fatal");

    assert!(matches!(
        err,
        PrecompileError::Fatal(msg) if msg.contains("database error")
    ));
}

#[test]
fn direct_call_nonzero_msg_value_is_ignored_by_current_precompile(
) -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(2));

    let output = call_transfer_precompile(
        &mut ctx,
        TransferCall { value: U256::from(1), ..TransferCall::new(&input) },
    )?;

    assert!(!output.reverted);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(8));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(2));
    Ok(())
}

#[test]
fn provider_dispatches_transfer_precompile() -> Result<(), Box<dyn core::error::Error>> {
    let mut ctx = context(U256::from(10), U256::ZERO);
    let input = transfer_input(FROM, TO, U256::from(2));
    let mut provider = ScrollPrecompileProvider::new_with_spec(ScrollSpecId::TSUKI);

    assert!(<ScrollPrecompileProvider as PrecompileProvider<ScrollContext<InMemoryDB>>>::contains(
        &provider,
        &TRANSFER_ADDRESS,
    ));
    assert!(<ScrollPrecompileProvider as PrecompileProvider<ScrollContext<InMemoryDB>>>::warm_addresses(
        &provider,
    )
        .any(|address| address == TRANSFER_ADDRESS));

    let result = provider
        .run(
            &mut ctx,
            &CallInputs {
                input: CallInput::Bytes(Bytes::from(input)),
                return_memory_offset: 0..0,
                gas_limit: TRANSFER_GAS_COST,
                bytecode_address: TRANSFER_ADDRESS,
                known_bytecode: None,
                target_address: TRANSFER_ADDRESS,
                caller: ALLOWED_CALLER,
                value: CallValue::Transfer(U256::ZERO),
                scheme: CallScheme::Call,
                is_static: false,
            },
        )
        .expect("provider run should not fatal")
        .expect("provider should handle transfer precompile address");

    assert_eq!(result.result, InstructionResult::Return);
    assert_eq!(balance(&mut ctx, FROM)?, U256::from(8));
    assert_eq!(balance(&mut ctx, TO)?, U256::from(2));
    Ok(())
}

#[test]
fn transfer_precompile_present_in_tsuki() {
    let precompiles = precompile::tsuki();

    assert!(precompiles.get(&TRANSFER_ADDRESS).is_some());
}

#[test]
fn transfer_precompile_absent_pre_tsuki() {
    assert!(precompile::galileo().get(&TRANSFER_ADDRESS).is_none());
}

#[test]
fn transfer_address_is_0xfd() {
    assert_eq!(TRANSFER_ADDRESS, u64_to_address(0xfd));
}
