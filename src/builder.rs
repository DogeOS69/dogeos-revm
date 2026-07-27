use crate::{
    chain::ScrollChainContext, evm::ScrollEvm, instructions::ScrollInstructions,
    transaction::ScrollTxTr, ScrollSpecId, ScrollTransaction,
};

use crate::gas::ScrollGasParams;
use revm::{
    context::{BlockEnv, Cfg, CfgEnv, JournalTr, TxEnv},
    context_interface::{cfg::GasParams, Block},
    database::EmptyDB,
    interpreter::interpreter::EthInterpreter,
    primitives::eip7825,
    state::EvmState,
    Context, Database, Journal, MainContext,
};

pub trait ScrollBuilder: Sized {
    type Context;

    fn build_scroll(
        self,
    ) -> ScrollEvm<Self::Context, (), ScrollInstructions<EthInterpreter, Self::Context>>;

    fn build_scroll_with_inspector<INSP>(
        self,
        inspector: INSP,
    ) -> ScrollEvm<Self::Context, INSP, ScrollInstructions<EthInterpreter, Self::Context>>;
}

impl<BLOCK, TX, CFG, DB, JOURNAL> ScrollBuilder
    for Context<BLOCK, TX, CFG, DB, JOURNAL, ScrollChainContext>
where
    BLOCK: Block,
    TX: ScrollTxTr,
    CFG: Cfg<Spec = ScrollSpecId>,
    DB: Database,
    JOURNAL: JournalTr<Database = DB, State = EvmState>,
{
    type Context = Self;

    fn build_scroll(
        self,
    ) -> ScrollEvm<Self::Context, (), ScrollInstructions<EthInterpreter, Self::Context>> {
        ScrollEvm::new(self, ())
    }

    fn build_scroll_with_inspector<INSP>(
        self,
        inspector: INSP,
    ) -> ScrollEvm<Self::Context, INSP, ScrollInstructions<EthInterpreter, Self::Context>> {
        ScrollEvm::new(self, inspector)
    }
}

/// Allows to build a default Scroll [`Context`].
pub trait DefaultScrollContext {
    fn scroll() -> ScrollContext<EmptyDB>;
}

impl DefaultScrollContext for ScrollContext<EmptyDB> {
    fn scroll() -> ScrollContext<EmptyDB> {
        let spec = ScrollSpecId::default();
        let cfg = CfgEnv::new_scroll(spec);

        Context::mainnet()
            .with_tx(ScrollTransaction::default())
            .with_cfg(cfg)
            .with_chain(ScrollChainContext::mainnet())
    }
}

/// Configures [`CfgEnv`] with the gas parameters and transaction limits for a Scroll hardfork.
///
/// Use this trait instead of assigning [`CfgEnv::spec`] directly or using the generic spec
/// setters. Those APIs do not update Scroll's gas parameters or the Tsuki transaction gas cap.
pub trait ScrollCfgExt {
    /// Creates a configuration for `spec` with all corresponding Scroll settings applied.
    fn new_scroll(spec: ScrollSpecId) -> Self;
    /// Sets the Scroll spec and updates its gas parameters and transaction limits accordingly.
    ///
    /// # Note
    ///
    /// This won't reset the `tx_gas_limit_cap` if it was already set, e.g. downgrading from a spec
    /// that has the cap to one that doesn't.
    fn set_scroll_spec(&mut self, spec: ScrollSpecId);
}

impl ScrollCfgExt for CfgEnv<ScrollSpecId> {
    fn new_scroll(spec: ScrollSpecId) -> Self {
        let mut cfg = CfgEnv::new_with_spec(spec);
        cfg.set_scroll_spec(spec);
        cfg
    }

    fn set_scroll_spec(&mut self, spec: ScrollSpecId) {
        self.spec = spec;
        self.set_gas_params(GasParams::new_scroll_spec(spec));

        if spec.is_enabled_in(ScrollSpecId::TSUKI) && self.tx_gas_limit_cap.is_none() {
            self.tx_gas_limit_cap = Some(eip7825::TX_GAS_LIMIT_CAP);
        }
    }
}

pub type ScrollContext<DB> = Context<
    BlockEnv,
    ScrollTransaction<TxEnv>,
    CfgEnv<ScrollSpecId>,
    DB,
    Journal<DB>,
    ScrollChainContext,
>;
