use crate::ScrollSpecId;
use alloy_evm::{precompiles::PrecompilesMap, Database};
use once_cell::race::OnceBox;
use revm::{
    context::Cfg,
    handler::PrecompileProvider,
    interpreter::{CallInputs, InterpreterResult},
    precompile::{self, secp256r1, Precompile, PrecompileError, PrecompileId, Precompiles},
    primitives::Address,
    Context, Journal,
};
use std::{boxed::Box, string::String};

mod blake2;
mod bn254;
mod hash;
mod modexp;
pub mod transfer;

/// Provides Scroll precompiles, modifying any relevant behaviour.
#[derive(Debug, Clone)]
pub struct ScrollPrecompileProvider {
    inner: PrecompilesMap,
    spec: ScrollSpecId,
}

impl ScrollPrecompileProvider {
    #[inline]
    pub fn new_with_spec(spec: ScrollSpecId) -> Self {
        let inner = match spec {
            ScrollSpecId::SHANGHAI => PrecompilesMap::from_static(pre_bernoulli()),
            ScrollSpecId::BERNOULLI | ScrollSpecId::CURIE | ScrollSpecId::DARWIN => {
                PrecompilesMap::from_static(bernoulli())
            }
            ScrollSpecId::EUCLID => PrecompilesMap::from_static(euclid()),
            ScrollSpecId::FEYNMAN => PrecompilesMap::from_static(feynman()),
            ScrollSpecId::GALILEO => PrecompilesMap::from_static(galileo()),
            ScrollSpecId::TSUKI => tsuki(),
        };
        Self { inner, spec }
    }

    /// Precompiles.
    #[inline]
    pub fn into_precompiles_map(self) -> PrecompilesMap {
        self.inner
    }
}

/// A helper function that creates a precompile that returns `PrecompileError::Other("Precompile not
/// implemented".into())` for a given address.
const fn precompile_not_implemented(id: PrecompileId, address: Address) -> Precompile {
    Precompile::new(id, address, |_input: &[u8], _gas_limit: u64| {
        Err(PrecompileError::Other("NotImplemented: Precompile not implemented".into()))
    })
}

/// Returns precompiles for Pre-Bernoulli spec.
pub(crate) fn pre_bernoulli() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = Precompiles::default();

        precompiles.extend([
            precompile::secp256k1::ECRECOVER,
            hash::sha256::SHANGHAI,
            hash::ripemd160::SHANGHAI,
            precompile::identity::FUN,
            modexp::BERNOULLI,
            precompile::bn254::add::ISTANBUL,
            precompile::bn254::mul::ISTANBUL,
            bn254::pair::BERNOULLI,
            blake2::SHANGHAI,
        ]);

        Box::new(precompiles)
    })
}

/// Returns precompiles for Bernoulli spec.
pub(crate) fn bernoulli() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = pre_bernoulli().clone();
        precompiles.extend([hash::sha256::BERNOULLI]);
        Box::new(precompiles)
    })
}

/// Returns precompiles for Euclid spec.
pub(crate) fn euclid() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = bernoulli().clone();
        precompiles.extend([secp256r1::P256VERIFY]);
        Box::new(precompiles)
    })
}

/// Returns precompiles for Feynman spec.
pub(crate) fn feynman() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = euclid().clone();
        precompiles.extend([bn254::pair::FEYNMAN]);
        Box::new(precompiles)
    })
}

pub(crate) fn galileo() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = feynman().clone();
        precompiles.extend([modexp::GALILEO, secp256r1::P256VERIFY_OSAKA]);
        Box::new(precompiles)
    })
}

pub(crate) fn tsuki() -> PrecompilesMap {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    let static_precompiles = INSTANCE.get_or_init(|| {
        let mut precompiles = galileo().clone();
        precompiles.extend([hash::ripemd160::TSUKI]);
        Box::new(precompiles)
    });

    PrecompilesMap::from_static(static_precompiles)
        .with_extended_precompiles([(transfer::ADDRESS, transfer::precompile().clone())])
}

impl<BlockEnv, TxEnv, CfgEnv, DB, Chain>
    PrecompileProvider<Context<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>, Chain>>
    for ScrollPrecompileProvider
where
    BlockEnv: revm::context::Block,
    TxEnv: revm::context::Transaction,
    CfgEnv: Cfg<Spec = ScrollSpecId>,
    DB: Database,
{
    type Output = InterpreterResult;

    #[inline]
    fn set_spec(&mut self, spec: <CfgEnv as Cfg>::Spec) -> bool {
        if spec == self.spec {
            return false;
        }
        *self = Self::new_with_spec(spec);
        true
    }

    #[inline]
    fn run(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>, Chain>,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        self.inner.run(context, inputs)
    }

    #[inline]
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        <PrecompilesMap as PrecompileProvider<
            Context<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>, Chain>,
        >>::warm_addresses(&self.inner)
    }

    #[inline]
    fn contains(&self, address: &Address) -> bool {
        <PrecompilesMap as PrecompileProvider<
            Context<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB>, Chain>,
        >>::contains(&self.inner, address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builder::{DefaultScrollContext, ScrollCfgExt},
        precompile::bn254::pair,
    };
    use alloy_evm::{
        precompiles::{Precompile, PrecompileInput},
        EvmInternals,
    };
    use revm::{
        context::{Cfg, CfgEnv},
        context_interface::cfg::gas,
        precompile::{u64_to_address, PrecompileError, PrecompileId, PrecompileResult},
        primitives::{hex, U256},
        Context,
    };
    use revm_primitives::eip7825;
    use std::{vec, vec::Vec};

    fn call_dyn_precompile(
        precompile: impl Precompile,
        caller: Address,
        address: Address,
        input: &[u8],
        gas: u64,
    ) -> PrecompileResult {
        let mut ctx = Context::scroll().with_cfg(CfgEnv::new_with_spec(ScrollSpecId::TSUKI));

        precompile.call(PrecompileInput {
            data: input,
            gas,
            caller,
            value: U256::ZERO,
            is_static: false,
            internals: EvmInternals::from_context(&mut ctx),
            target_address: address,
            bytecode_address: address,
        })
    }

    fn precompiles_for_spec(spec: ScrollSpecId) -> PrecompilesMap {
        ScrollPrecompileProvider::new_with_spec(spec).into_precompiles_map()
    }

    fn expected_precompile_addresses(spec: ScrollSpecId) -> Vec<Address> {
        let mut addresses = (1..=9).map(u64_to_address).collect::<Vec<_>>();

        if spec >= ScrollSpecId::EUCLID {
            addresses.push(u64_to_address(256));
        }
        if spec >= ScrollSpecId::TSUKI {
            addresses.push(transfer::ADDRESS);
        }

        addresses
    }

    fn expected_precompile_id(address: Address) -> PrecompileId {
        match address {
            _ if address == u64_to_address(1) => PrecompileId::EcRec,
            _ if address == hash::sha256::ADDRESS => PrecompileId::Sha256,
            _ if address == hash::ripemd160::ADDRESS => PrecompileId::Ripemd160,
            _ if address == u64_to_address(4) => PrecompileId::Identity,
            _ if address == modexp::ADDRESS => PrecompileId::ModExp,
            _ if address == u64_to_address(6) => PrecompileId::Bn254Add,
            _ if address == u64_to_address(7) => PrecompileId::Bn254Mul,
            _ if address == pair::ADDRESS => PrecompileId::Bn254Pairing,
            _ if address == blake2::ADDRESS => PrecompileId::Blake2F,
            _ if address == u64_to_address(256) => PrecompileId::P256Verify,
            _ if address == transfer::ADDRESS => transfer::ID.clone(),
            _ => panic!("unexpected precompile address: {address}"),
        }
    }

    fn expected_zero_input_gas(spec: ScrollSpecId, address: Address) -> Option<u64> {
        match address {
            _ if address == u64_to_address(1) => Some(3_000),
            _ if address == hash::sha256::ADDRESS && spec >= ScrollSpecId::BERNOULLI => Some(60),
            _ if address == hash::ripemd160::ADDRESS && spec >= ScrollSpecId::TSUKI => Some(600),
            _ if address == u64_to_address(4) => Some(15),
            _ if address == modexp::ADDRESS && spec >= ScrollSpecId::GALILEO => Some(500),
            _ if address == modexp::ADDRESS => Some(200),
            _ if address == u64_to_address(6) => Some(150),
            _ if address == u64_to_address(7) => Some(6_000),
            _ if address == pair::ADDRESS => Some(45_000),
            _ if address == u64_to_address(256) && spec >= ScrollSpecId::GALILEO => Some(6_900),
            _ if address == u64_to_address(256) => Some(3_450),
            _ => None,
        }
    }

    fn expected_not_implemented(spec: ScrollSpecId, address: Address) -> bool {
        matches!(
            address,
            _ if address == hash::sha256::ADDRESS && spec < ScrollSpecId::BERNOULLI
        ) || matches!(
            address,
            _ if address == hash::ripemd160::ADDRESS && spec < ScrollSpecId::TSUKI
        ) || address == blake2::ADDRESS
    }

    fn assert_precompile_zero_input_gas(
        spec: ScrollSpecId,
        precompiles: &PrecompilesMap,
        address: Address,
        expected_gas: u64,
    ) {
        let precompile =
            precompiles.get(&address).expect("expected precompile should be installed");

        let output = call_dyn_precompile(precompile, Address::ZERO, address, &[], expected_gas)
            .unwrap_or_else(|err| panic!("{spec:?} precompile {address} failed: {err:?}"));

        assert_eq!(output.gas_used, expected_gas, "{spec:?} precompile {address}");

        if expected_gas > 0 {
            let precompile =
                precompiles.get(&address).expect("expected precompile should be installed");
            let err =
                call_dyn_precompile(precompile, Address::ZERO, address, &[], expected_gas - 1)
                    .expect_err("precompile should reject gas below its expected cost");
            assert_eq!(err, PrecompileError::OutOfGas, "{spec:?} precompile {address}");
        }
    }

    fn assert_precompile_not_implemented(
        spec: ScrollSpecId,
        precompiles: &PrecompilesMap,
        address: Address,
    ) {
        let precompile =
            precompiles.get(&address).expect("expected precompile should be installed");
        let err = call_dyn_precompile(precompile, Address::ZERO, address, &[], u64::MAX)
            .expect_err("precompile should be installed as a disabled placeholder");

        assert!(
            matches!(&err, PrecompileError::Other(msg) if msg.contains("NotImplemented")),
            "{spec:?} precompile {address} should be a disabled placeholder, got {err:?}"
        );
    }

    #[test]
    fn spec_activation_matrix_matches_scroll_plan() {
        for spec in [
            ScrollSpecId::SHANGHAI,
            ScrollSpecId::BERNOULLI,
            ScrollSpecId::CURIE,
            ScrollSpecId::DARWIN,
            ScrollSpecId::EUCLID,
            ScrollSpecId::FEYNMAN,
            ScrollSpecId::GALILEO,
            ScrollSpecId::TSUKI,
        ] {
            let cfg = CfgEnv::new_scroll(spec);

            let expected_eip7702 = spec >= ScrollSpecId::EUCLID;
            let expected_eip7623 = spec >= ScrollSpecId::FEYNMAN;
            let expected_eip7825 = spec >= ScrollSpecId::TSUKI;

            assert_eq!(
                cfg.gas_params.tx_eip7702_per_empty_account_cost(),
                if expected_eip7702 { revm_primitives::eip7702::PER_EMPTY_ACCOUNT_COST } else { 0 },
                "{spec:?} EIP-7702 gas override"
            );
            assert_eq!(
                cfg.gas_params.tx_floor_cost_per_token(),
                if expected_eip7623 { gas::TOTAL_COST_FLOOR_PER_TOKEN } else { 0 },
                "{spec:?} EIP-7623 floor token gas override"
            );
            assert_eq!(
                cfg.gas_params.tx_floor_cost_base_gas(),
                if expected_eip7623 { 21_000 } else { 0 },
                "{spec:?} EIP-7623 floor base gas override"
            );
            assert_eq!(
                cfg.tx_gas_limit_cap,
                if expected_eip7825 { Some(eip7825::TX_GAS_LIMIT_CAP) } else { None },
                "{spec:?} EIP-7825 cap override"
            );
            assert_eq!(
                cfg.tx_gas_limit_cap(),
                if expected_eip7825 { eip7825::TX_GAS_LIMIT_CAP } else { u64::MAX },
                "{spec:?} EIP-7825 effective cap"
            );

            let precompiles = precompiles_for_spec(spec);
            let mut actual_addresses = precompiles.addresses().copied().collect::<Vec<_>>();
            actual_addresses.sort_unstable();

            let mut expected_addresses = expected_precompile_addresses(spec);
            expected_addresses.sort_unstable();

            assert_eq!(actual_addresses, expected_addresses, "{spec:?} precompile set");

            for address in expected_addresses {
                let precompile =
                    precompiles.get(&address).expect("expected precompile should be installed");
                assert_eq!(
                    precompile.precompile_id(),
                    &expected_precompile_id(address),
                    "{spec:?} precompile id at {address}"
                );

                if let Some(expected_gas) = expected_zero_input_gas(spec, address) {
                    assert_precompile_zero_input_gas(spec, &precompiles, address, expected_gas);
                } else if expected_not_implemented(spec, address) {
                    assert_precompile_not_implemented(spec, &precompiles, address);
                }
            }

            if spec >= ScrollSpecId::TSUKI {
                let precompile = precompiles
                    .get(&transfer::ADDRESS)
                    .expect("transfer precompile should be installed in TSUKI");
                let input = vec![0; 96];
                let output = call_dyn_precompile(
                    precompile,
                    transfer::NATIVE_DOGE_TOKEN_ADDRESS,
                    transfer::ADDRESS,
                    &input,
                    transfer::GAS_COST,
                )
                .expect("transfer precompile should accept exact gas");
                assert_eq!(output.gas_used, transfer::GAS_COST);

                let precompile = precompiles
                    .get(&transfer::ADDRESS)
                    .expect("transfer precompile should be installed in TSUKI");
                let err = call_dyn_precompile(
                    precompile,
                    transfer::NATIVE_DOGE_TOKEN_ADDRESS,
                    transfer::ADDRESS,
                    &input,
                    transfer::GAS_COST - 1,
                )
                .expect_err("transfer precompile should reject gas below its expected cost");
                assert_eq!(err, PrecompileError::OutOfGas);
            }
        }
    }

    #[test]
    fn test_ripemd160_enabled_only_from_tsuki() {
        let input = [];
        let expected =
            hex::decode("0000000000000000000000009c1185a5c5e9fc54612808977ee8f548b2258d31")
                .unwrap();

        let precompile =
            galileo().get(&hash::ripemd160::ADDRESS).expect("precompile exists before TSUKI");
        let outcome = precompile.execute(&input, u64::MAX);
        assert!(matches!(
            outcome,
            Err(PrecompileError::Other(msg)) if msg.contains("NotImplemented")
        ));

        let precompiles = tsuki();
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");
        let outcome = call_dyn_precompile(
            precompile,
            Address::ZERO,
            hash::ripemd160::ADDRESS,
            &input,
            u64::MAX,
        )
        .expect("call succeeds");
        assert_eq!(outcome.bytes.as_ref(), expected.as_slice());
    }

    #[test]
    fn test_tsuki_ripemd160_accepts_32_byte_input() {
        let input = vec![0xff; hash::ripemd160::TSUKI_LEN_LIMIT];
        let precompiles = tsuki();
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");

        let outcome = call_dyn_precompile(
            precompile,
            Address::ZERO,
            hash::ripemd160::ADDRESS,
            &input,
            u64::MAX,
        );

        assert!(outcome.is_ok(), "32-byte input should be accepted");
    }

    #[test]
    fn test_tsuki_ripemd160_rejects_33_byte_input() {
        let input = vec![0xff; hash::ripemd160::TSUKI_LEN_LIMIT + 1];
        let precompiles = tsuki();
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");

        let outcome = call_dyn_precompile(
            precompile,
            Address::ZERO,
            hash::ripemd160::ADDRESS,
            &input,
            u64::MAX,
        );

        assert!(matches!(
            outcome,
            Err(PrecompileError::Other(msg)) if msg.contains("Ripemd160InputOverflow")
        ));
    }

    #[test]
    fn test_bn128_large_input() {
        // test case copied from geth
        let input = hex::decode("00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d").unwrap();

        let expected =
            hex::decode("0000000000000000000000000000000000000000000000000000000000000001")
                .unwrap();

        // Euclid version should reject this input
        let precompile = euclid().get(&pair::ADDRESS).expect("precompile exists");
        let outcome = precompile.execute(&input, u64::MAX);
        assert!(outcome.is_err());

        // Feynman version should accept this input
        let precompile = feynman().get(&pair::ADDRESS).expect("precompile exists");
        let outcome = precompile.execute(&input, u64::MAX).expect("call succeeds");
        assert_eq!(outcome.bytes, expected);
    }
}
