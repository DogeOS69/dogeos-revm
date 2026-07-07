use crate::{
    evm::ScrollEvm, instructions::ScrollInstructions, l1block::L1BlockInfo,
    transaction::ScrollTxTr, ScrollSpecId, ScrollTransaction,
};

use revm::{
    context::{BlockEnv, Cfg, CfgEnv, JournalTr, TxEnv},
    context_interface::Block,
    database::EmptyDB,
    interpreter::interpreter::EthInterpreter,
    primitives::{eip7825, Address},
    state::EvmState,
    Context, Database, Journal, MainContext,
};

pub trait ScrollBuilder: Sized {
    type Context;

    fn build_scroll(
        self,
        allow_transfer_caller: Option<Address>,
    ) -> ScrollEvm<Self::Context, (), ScrollInstructions<EthInterpreter, Self::Context>>;

    fn build_scroll_with_inspector<INSP>(
        self,
        inspector: INSP,
        allow_transfer_caller: Option<Address>,
    ) -> ScrollEvm<Self::Context, INSP, ScrollInstructions<EthInterpreter, Self::Context>>;
}

impl<BLOCK, TX, CFG, DB, JOURNAL> ScrollBuilder
    for Context<BLOCK, TX, CFG, DB, JOURNAL, L1BlockInfo>
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
        allow_transfer_caller: Option<Address>,
    ) -> ScrollEvm<Self::Context, (), ScrollInstructions<EthInterpreter, Self::Context>> {
        ScrollEvm::new(self, (), allow_transfer_caller)
    }

    fn build_scroll_with_inspector<INSP>(
        self,
        inspector: INSP,
        allow_transfer_caller: Option<Address>,
    ) -> ScrollEvm<Self::Context, INSP, ScrollInstructions<EthInterpreter, Self::Context>> {
        ScrollEvm::new(self, inspector, allow_transfer_caller)
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
        if spec >= ScrollSpecId::EUCLID {
            cfg = cfg.enable_eip_7702();
        }
        if spec >= ScrollSpecId::FEYNMAN {
            cfg = cfg.enable_eip_7623();
        }

        Context::mainnet()
            .with_tx(ScrollTransaction::default())
            .with_cfg(cfg)
            .with_chain(L1BlockInfo::default())
    }
}

/// Activates specific EIP's for Euclid.
pub trait EuclidEipActivations {
    /// Activates EIP-7702 if the spec is at least at Euclid.
    fn maybe_with_eip_7702(self) -> Self;
}

/// Activates specific EIP's for Feynman.
pub trait FeynmanEipActivations: EuclidEipActivations {
    /// Activates EIP-7623 if the spec is at least at Feynman.
    fn maybe_with_eip_7623(self) -> Self;
}

/// Activates specific EIP's for Tsuki.
pub trait TsukiEipActivations: FeynmanEipActivations {
    /// Activates EIP-7825 if the spec is at least at Tsuki.
    fn maybe_with_eip_7825(self) -> Self;
}

impl EuclidEipActivations for CfgEnv<ScrollSpecId> {
    fn maybe_with_eip_7702(mut self) -> Self {
        if self.spec.is_enabled_in(ScrollSpecId::EUCLID) {
            self = self.enable_eip_7702();
        }
        self
    }
}

impl<DB: Database> EuclidEipActivations for ScrollContext<DB> {
    fn maybe_with_eip_7702(mut self) -> Self {
        self.cfg = self.cfg.maybe_with_eip_7702();
        self
    }
}

impl FeynmanEipActivations for CfgEnv<ScrollSpecId> {
    fn maybe_with_eip_7623(mut self) -> Self {
        if self.spec.is_enabled_in(ScrollSpecId::FEYNMAN) {
            self = self.enable_eip_7623();
        }
        self
    }
}

impl<DB: Database> FeynmanEipActivations for ScrollContext<DB> {
    fn maybe_with_eip_7623(mut self) -> Self {
        self.cfg = self.cfg.maybe_with_eip_7623();
        self
    }
}

impl TsukiEipActivations for CfgEnv<ScrollSpecId> {
    fn maybe_with_eip_7825(mut self) -> Self {
        if self.spec.is_enabled_in(ScrollSpecId::TSUKI) && self.tx_gas_limit_cap.is_none() {
            self.tx_gas_limit_cap = Some(eip7825::TX_GAS_LIMIT_CAP);
        }
        self
    }
}

impl<DB: Database> TsukiEipActivations for ScrollContext<DB> {
    fn maybe_with_eip_7825(mut self) -> Self {
        self.cfg = self.cfg.maybe_with_eip_7825();
        self
    }
}

pub type ScrollContext<DB> =
    Context<BlockEnv, ScrollTransaction<TxEnv>, CfgEnv<ScrollSpecId>, DB, Journal<DB>, L1BlockInfo>;
