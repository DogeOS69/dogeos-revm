use crate::l1block::L1BlockInfo;

/// Chain-wide Scroll execution policy that is independent of a transaction or block.
///
/// This is operator/chain policy, not part of a Scroll hardfork: it is not derived from
/// `cfg.spec` and must be configured explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollChainPolicy {
    /// Whether fee-charged transactions must reserve an L1 data-fee buffer.
    ///
    /// This applies only to transactions that are charged an L1 data fee (neither L1 messages
    /// nor system transactions). When enabled, pre-execution requires the caller's balance to
    /// cover exactly 2x the L1 data fee (1x charged + 1x buffer); only the actual 1x fee is
    /// deducted.
    pub require_l1_data_fee_buffer: bool,
}

impl ScrollChainPolicy {
    /// Returns the mainnet policy, which does not require an L1 data-fee buffer by default.
    pub const fn mainnet() -> Self {
        Self { require_l1_data_fee_buffer: false }
    }

    /// Sets the L1 data-fee buffer requirement.
    pub const fn with_l1_data_fee_buffer(mut self, require: bool) -> Self {
        self.require_l1_data_fee_buffer = require;
        self
    }
}

/// Scroll-specific execution state shared by transactions in an EVM context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScrollChainContext {
    /// L1 fee parameters loaded from the L1 gas price oracle.
    pub l1_block_info: L1BlockInfo,
    /// Chain-wide execution policy.
    pub policy: ScrollChainPolicy,
}

impl ScrollChainContext {
    /// Creates a chain context from explicit L1 fee parameters and policy.
    pub fn new(l1_block_info: L1BlockInfo, policy: ScrollChainPolicy) -> Self {
        Self { l1_block_info, policy }
    }

    /// Creates the default mainnet chain context.
    pub fn mainnet() -> Self {
        Self { l1_block_info: L1BlockInfo::default(), policy: ScrollChainPolicy::mainnet() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_policy_keeps_l1_data_fee_buffer_disabled() {
        assert!(!ScrollChainPolicy::mainnet().require_l1_data_fee_buffer);
        assert!(!ScrollChainContext::mainnet().policy.require_l1_data_fee_buffer);
    }

    #[test]
    fn l1_data_fee_buffer_policy_can_be_enabled_explicitly() {
        let policy = ScrollChainPolicy::mainnet().with_l1_data_fee_buffer(true);

        assert!(policy.require_l1_data_fee_buffer);
    }
}
