use crate::ScrollSpecId;
use revm::context_interface::cfg::{gas, GasId, GasParams};
use revm_primitives::{eip7702, hardfork::SpecId};

pub trait ScrollGasParams {
    fn new_scroll_spec(spec: ScrollSpecId) -> GasParams {
        let mut params = GasParams::new_spec(SpecId::SHANGHAI);

        if spec.is_enabled_in(ScrollSpecId::EUCLID) {
            params.override_gas([
                (GasId::tx_eip7702_per_empty_account_cost(), eip7702::PER_EMPTY_ACCOUNT_COST),
                // Note: uncommenting this when upgrade revm
                // (
                //     GasId::tx_eip7702_auth_refund(),
                //     eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST,
                // ),
            ]);
        }

        if spec.is_enabled_in(ScrollSpecId::FEYNMAN) {
            params.override_gas([
                (GasId::tx_floor_cost_per_token(), gas::TOTAL_COST_FLOOR_PER_TOKEN),
                (GasId::tx_floor_cost_base_gas(), 21_000),
                // Note: uncommenting this when upgrade revm
                // (GasId::tx_floor_token_zero_byte_multiplier(), 1),
            ]);
        }

        params
    }
}

impl ScrollGasParams for GasParams {}
