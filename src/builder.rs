use crate::{
    chain::ScrollChainContext, evm::ScrollEvm, gas::scroll_gas_params,
    instructions::ScrollInstructions, transaction::ScrollTxTr, ScrollSpecId, ScrollTransaction,
};

use revm::{
    context::{BlockEnv, Cfg, CfgEnv, JournalTr, TxEnv},
    context_interface::Block,
    database::EmptyDB,
    interpreter::interpreter::EthInterpreter,
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
        let mut cfg = CfgEnv::new_with_spec(spec);
        cfg.set_gas_params(scroll_gas_params(spec));

        Context::mainnet()
            .with_tx(ScrollTransaction::default())
            .with_cfg(cfg)
            .with_chain(ScrollChainContext::mainnet())
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
