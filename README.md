# scroll-revm

scroll-revm is an implementation of Scroll's EVM, utilizing the [revm](https://github.com/bluealloy/revm) library—a Rust-based Ethereum Virtual Machine (EVM) implementation. This project adopts an SDK-like pattern, allowing us to override and extend the EVM functionality with Scroll specific logic cleanly and efficiently.

## Overview

The goal of scroll-revm is to provide a flexible and maintainable solution for integrating [reth](https://github.com/paradigmxyz/reth) with the scroll-revm EVM, offering a seamless way to adapt and evolve the EVM implementation as required by Scroll's rollup environment. By leveraging revm's public API, scroll-revm makes it possible to introduce custom modifications and optimizations specific to Scroll, while maintaining compatibility with the original revm codebase.

## Features

- **Rust-based EVM:** Built on top of the revm EVM implementation, offering efficient performance and compatibility with the Ethereum ecosystem.
- **SDK-Like Structure:** Provides an easy-to-use interface for overriding and extending specific components of Scroll's EVM.
- **Modular and Maintainable:** Designed to keep the codebase clean, modular, and easy to maintain, making future updates and changes straightforward.

## Rust toolchains

CI uses Rust **1.93.0** for Clippy, documentation, feature checks, and the RISC-V
`no_std` check, matching the `dogeos-reth` execution-node CI. Tests run on 1.93.0,
**1.96.1** (the SBV 1.96 host toolchain), and **nightly-2026-03-17** (the
`zkvm-prover` toolchain). Rustfmt also uses nightly-2026-03-17.

Use these explicit versions when reproducing CI, for example:

```sh
RUSTFLAGS=-Dwarnings cargo +1.93.0 clippy --locked --workspace --all-targets --all-features
cargo +1.93.0 test --locked --all-features
cargo +nightly-2026-03-17 fmt --all --check
```

Update the pins deliberately when the consuming repositories change toolchains;
floating `stable` or `nightly` channels can introduce new lints independently of
the execution/prover build environments.

## TsukiV2 BLOCKHASH pricing

TsukiV2 activates [EIP-7709](https://eips.ethereum.org/EIPS/eip-7709). An in-window
`BLOCKHASH` retains its 20 gas base cost and additionally pays the ordinary SLOAD
cost for the EIP-2935 history slot: 2120 gas total when cold, or 120 when warm.
The slot shares warming and revert semantics with SLOAD and transaction access
lists. The history account's warmth is unchanged, and no contract code is executed.
This follows EIP-7709's rule that only SLOAD effects apply, including when a client
internally resolves the value through the system contract's `get` entry point:
that internal resolution must not leave the account warm as a normal CALL would.
Queries outside the last 256 ancestors still return zero for 20 gas without a
storage access. Earlier forks, including Tsuki, retain the fixed 20 gas charge.

This uses the `TSUKI_V2` spec selection; no activation timestamp is added in
this crate. Chains must have EIP-2935 history populated for at least 256 blocks
before activation, or enable both EIPs at genesis as permitted by EIP-7709.
