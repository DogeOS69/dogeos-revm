use crate::ScrollSpecId;
use revm::context_interface::cfg::{gas, GasId, GasParams};
use revm_primitives::eip7702;

pub trait ScrollGasParams {
    /// Returns the gas parameters for a Scroll hardfork.
    ///
    /// The Ethereum baseline follows the `ScrollSpecId` mapping. Scroll hardfork-specific
    /// changes are applied below because they do not have one-to-one Ethereum spec IDs.
    fn new_scroll_spec(spec: ScrollSpecId) -> GasParams {
        let mut params = GasParams::new_spec(spec.into());

        if spec.is_enabled_in(ScrollSpecId::EUCLID) {
            // SCROLL DIVERGENCE: all current Scroll specs use the Ethereum Shanghai baseline,
            // but Euclid activates EIP-7702's authorization-list intrinsic gas.
            params.override_gas([
                (GasId::tx_eip7702_per_empty_account_cost(), eip7702::PER_EMPTY_ACCOUNT_COST),
                (
                    GasId::tx_eip7702_auth_refund(),
                    eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST,
                ),
            ]);
        }

        if spec.is_enabled_in(ScrollSpecId::FEYNMAN) {
            // SCROLL DIVERGENCE: Feynman activates EIP-7623 on top of the Shanghai baseline.
            params.override_gas([
                (GasId::tx_floor_cost_per_token(), gas::TOTAL_COST_FLOOR_PER_TOKEN),
                (GasId::tx_floor_cost_base_gas(), 21_000),
                // Re-evaluate this override when revm exposes the zero-byte multiplier.
                // (GasId::tx_floor_token_zero_byte_multiplier(), 1),
            ]);
        }

        params
    }
}

impl ScrollGasParams for GasParams {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eip7702_auth_refund_per_spec() {
        // Pre-Euclid specs do not activate EIP-7702: no authorization refund.
        let params = GasParams::new_scroll_spec(ScrollSpecId::DARWIN);
        assert_eq!(params.tx_eip7702_auth_refund(), 0);

        // Euclid and later refund the existing-authority delta while keeping the
        // per-empty-account intrinsic authorization cost.
        for spec in [
            ScrollSpecId::EUCLID,
            ScrollSpecId::FEYNMAN,
            ScrollSpecId::GALILEO,
            ScrollSpecId::TSUKI,
        ] {
            let params = GasParams::new_scroll_spec(spec);
            assert_eq!(
                params.tx_eip7702_auth_refund(),
                eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST
            );
            assert_eq!(params.tx_eip7702_auth_refund(), 12_500);
            assert_eq!(params.tx_eip7702_per_empty_account_cost(), 25_000);
        }
    }
}
