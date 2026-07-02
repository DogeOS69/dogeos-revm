use crate::l1block::L1BlockInfo;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollChainPolicy {
    pub require_l1_data_fee_buffer: bool,
}

impl ScrollChainPolicy {
    pub const fn mainnet() -> Self {
        Self { require_l1_data_fee_buffer: false }
    }

    pub const fn with_l1_data_fee_buffer(mut self, require: bool) -> Self {
        self.require_l1_data_fee_buffer = require;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScrollChainContext {
    pub l1_block_info: L1BlockInfo,
    pub policy: ScrollChainPolicy,
}

impl ScrollChainContext {
    pub fn new(l1_block_info: L1BlockInfo, policy: ScrollChainPolicy) -> Self {
        Self { l1_block_info, policy }
    }

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
