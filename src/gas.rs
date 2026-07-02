use crate::ScrollSpecId;
use revm::{
    context_interface::cfg::{gas, GasId, GasParams},
    primitives::{eip7702, hardfork::SpecId},
};

pub fn scroll_gas_params(spec: ScrollSpecId) -> GasParams {
    let mut params = GasParams::new_spec(SpecId::SHANGHAI);

    if spec.is_enabled_in(ScrollSpecId::EUCLID) {
        params.override_gas([
            (GasId::tx_eip7702_per_empty_account_cost(), eip7702::PER_EMPTY_ACCOUNT_COST),
            (
                GasId::tx_eip7702_auth_refund(),
                eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST,
            ),
        ]);
    }

    if spec.is_enabled_in(ScrollSpecId::FEYNMAN) {
        params.override_gas([
            (GasId::tx_floor_cost_per_token(), gas::TOTAL_COST_FLOOR_PER_TOKEN),
            (GasId::tx_floor_cost_base_gas(), 21_000),
            (GasId::tx_floor_token_zero_byte_multiplier(), 1),
        ]);
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eip7702_gas_values_match_scroll_schedule() {
        assert_eq!(eip7702::PER_EMPTY_ACCOUNT_COST, 25_000);
        assert_eq!(eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST, 12_500);
    }

    #[test]
    fn feynman_enables_scroll_gas_overrides() {
        let params = scroll_gas_params(ScrollSpecId::FEYNMAN);

        assert_eq!(
            params.get(GasId::tx_eip7702_per_empty_account_cost()),
            eip7702::PER_EMPTY_ACCOUNT_COST
        );
        assert_eq!(
            params.get(GasId::tx_eip7702_auth_refund()),
            eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST
        );
        assert_eq!(params.get(GasId::tx_floor_cost_per_token()), gas::TOTAL_COST_FLOOR_PER_TOKEN);
        assert_eq!(params.get(GasId::tx_floor_cost_base_gas()), 21_000);
        assert_eq!(params.get(GasId::tx_floor_token_zero_byte_multiplier()), 1);
    }

    #[test]
    fn euclid_enables_eip7702_without_eip7623_floor_gas() {
        let params = scroll_gas_params(ScrollSpecId::EUCLID);

        assert_eq!(
            params.get(GasId::tx_eip7702_per_empty_account_cost()),
            eip7702::PER_EMPTY_ACCOUNT_COST
        );
        assert_eq!(
            params.get(GasId::tx_eip7702_auth_refund()),
            eip7702::PER_EMPTY_ACCOUNT_COST - eip7702::PER_AUTH_BASE_COST
        );
        assert_eq!(params.get(GasId::tx_floor_cost_per_token()), 0);
    }

    #[test]
    fn pre_euclid_does_not_enable_eip7702_or_eip7623_gas() {
        let params = scroll_gas_params(ScrollSpecId::DARWIN);

        assert_eq!(params.get(GasId::tx_eip7702_per_empty_account_cost()), 0);
        assert_eq!(params.get(GasId::tx_eip7702_auth_refund()), 0);
        assert_eq!(params.get(GasId::tx_floor_cost_per_token()), 0);
    }
}
