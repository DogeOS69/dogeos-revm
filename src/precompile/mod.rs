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
pub(crate) mod transfer;

/// Provides Scroll precompiles, modifying any relevant behaviour.
#[derive(Debug, Clone)]
pub struct ScrollPrecompileProvider {
    inner: PrecompilesMap,
    spec: ScrollSpecId,
    allow_transfer_caller: Option<Address>,
}

impl ScrollPrecompileProvider {
    #[inline]
    pub fn new_with_spec(spec: ScrollSpecId, allow_transfer_caller: Option<Address>) -> Self {
        let inner = match spec {
            ScrollSpecId::SHANGHAI => PrecompilesMap::from_static(pre_bernoulli()),
            ScrollSpecId::BERNOULLI | ScrollSpecId::CURIE | ScrollSpecId::DARWIN => {
                PrecompilesMap::from_static(bernoulli())
            }
            ScrollSpecId::EUCLID => PrecompilesMap::from_static(euclid()),
            ScrollSpecId::FEYNMAN => PrecompilesMap::from_static(feynman()),
            ScrollSpecId::GALILEO => PrecompilesMap::from_static(galileo()),
            ScrollSpecId::TSUKI => tsuki(
                allow_transfer_caller
                    .expect("allow_transfer_caller must be provided for TSUKI spec"),
            ),
        };
        Self { inner, spec, allow_transfer_caller }
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

pub(crate) fn tsuki(allow_transfer_caller: Address) -> PrecompilesMap {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    let static_precompiles = INSTANCE.get_or_init(|| {
        let mut precompiles = galileo().clone();
        precompiles.extend([hash::ripemd160::TSUKI]);
        Box::new(precompiles)
    });

    PrecompilesMap::from_static(static_precompiles)
        .with_extended_precompiles([(transfer::ADDRESS, transfer::tsuki(allow_transfer_caller))])
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
        *self = Self::new_with_spec(spec, self.allow_transfer_caller);
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
    use crate::{builder::DefaultScrollContext, precompile::bn254::pair};
    use alloy_evm::{
        precompiles::{Precompile, PrecompileInput},
        EvmInternals,
    };
    use revm::{
        context::CfgEnv,
        precompile::{PrecompileError, PrecompileResult},
        primitives::{hex, U256},
        Context,
    };
    use std::vec;

    fn call_dyn_precompile(
        precompile: impl Precompile,
        address: Address,
        input: &[u8],
        gas: u64,
    ) -> PrecompileResult {
        let mut ctx = Context::scroll().with_cfg(CfgEnv::new_with_spec(ScrollSpecId::TSUKI));

        precompile.call(PrecompileInput {
            data: input,
            gas,
            caller: Address::ZERO,
            value: U256::ZERO,
            is_static: false,
            internals: EvmInternals::from_context(&mut ctx),
            target_address: address,
            bytecode_address: address,
        })
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

        let precompiles = tsuki(Address::ZERO);
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");
        let outcome = call_dyn_precompile(precompile, hash::ripemd160::ADDRESS, &input, u64::MAX)
            .expect("call succeeds");
        assert_eq!(outcome.bytes.as_ref(), expected.as_slice());
    }

    #[test]
    fn test_tsuki_ripemd160_accepts_32_byte_input() {
        let input = vec![0xff; hash::ripemd160::TSUKI_LEN_LIMIT];
        let precompiles = tsuki(Address::ZERO);
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");

        let outcome = call_dyn_precompile(precompile, hash::ripemd160::ADDRESS, &input, u64::MAX);

        assert!(outcome.is_ok(), "32-byte input should be accepted");
    }

    #[test]
    fn test_tsuki_ripemd160_rejects_33_byte_input() {
        let input = vec![0xff; hash::ripemd160::TSUKI_LEN_LIMIT + 1];
        let precompiles = tsuki(Address::ZERO);
        let precompile =
            precompiles.get(&hash::ripemd160::ADDRESS).expect("precompile exists in TSUKI");

        let outcome = call_dyn_precompile(precompile, hash::ripemd160::ADDRESS, &input, u64::MAX);

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
