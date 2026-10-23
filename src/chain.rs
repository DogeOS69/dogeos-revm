use crate::l1block::L1BlockInfo;

/// Scroll-specific execution state shared by transactions in an EVM context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScrollChainContext {
    /// L1 fee parameters loaded from the L1 gas price oracle.
    pub l1_block_info: L1BlockInfo,
}

impl ScrollChainContext {
    /// Creates a chain context from explicit L1 fee parameters.
    pub fn new(l1_block_info: L1BlockInfo) -> Self {
        Self { l1_block_info }
    }

    /// Creates the default mainnet chain context.
    pub fn mainnet() -> Self {
        Self::default()
    }
}
