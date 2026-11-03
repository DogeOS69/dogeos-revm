use crate::ScrollSpecId;

use revm::{
    context::JournalTr,
    inspector::JournalExt,
    primitives::{address, Address, U256},
    Database,
};

// CONSTANTS
// ================================================================================================

/// The cost of a zero byte in calldata.
const ZERO_BYTE_COST: u64 = 4;

/// The cost of a non-zero byte in calldata.
const NON_ZERO_BYTE_COST: u64 = 16;

/// The extra cost of committing a transaction on L1.
const TX_L1_COMMIT_EXTRA_COST: U256 = U256::from_limbs([64u64, 0, 0, 0]);

/// The precision used for L1 fee calculations.
pub const TX_L1_FEE_PRECISION: u64 = 1_000_000_000u64;
pub const TX_L1_FEE_PRECISION_U256: U256 = U256::from_limbs([TX_L1_FEE_PRECISION, 0, 0, 0]);

/// The L1 gas price oracle address.
pub const L1_GAS_PRICE_ORACLE_ADDRESS: Address =
    address!("5300000000000000000000000000000000000002");

/// The L1 base fee storage slot.
pub const L1_BASE_FEE_SLOT: U256 = U256::from_limbs([1u64, 0, 0, 0]);

/// The L1 fee overhead storage slot.
pub const L1_OVERHEAD_SLOT: U256 = U256::from_limbs([2u64, 0, 0, 0]);

/// The L1 fee scalar storage slot.
pub const L1_SCALAR_SLOT: U256 = U256::from_limbs([3u64, 0, 0, 0]);

/// The L1 blob base fee storage slot.
pub const L1_BLOB_BASE_FEE_SLOT: U256 = U256::from_limbs([5u64, 0, 0, 0]);

/// The L1 commit scalar storage slot.
///
/// Post-FEYNMAN this represents the exec_scalar.
pub const L1_COMMIT_SCALAR_SLOT: U256 = U256::from_limbs([6u64, 0, 0, 0]);

/// The L1 blob scalar storage slot.
pub const L1_BLOB_SCALAR_SLOT: U256 = U256::from_limbs([7u64, 0, 0, 0]);

/// The compression penalty threshold storage slot.
pub const PENALTY_THRESHOLD_SLOT: U256 = U256::from_limbs([9u64, 0, 0, 0]);

/// The compression penalty factor storage slot.
pub const PENALTY_FACTOR_SLOT: U256 = U256::from_limbs([10u64, 0, 0, 0]);

const U64_MAX: U256 = U256::from_limbs([u64::MAX, 0, 0, 0]);
const U96_MAX: U256 = U256::from_limbs([u64::MAX, u32::MAX as u64, 0, 0]);

// L1 FEE ERROR
// ================================================================================================

/// An error returned when the L1 fee of a transaction cannot be calculated from its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum L1FeeError {
    /// An L1 gas price oracle value required by the spec is not set.
    MissingOracleValue,
    /// The compression ratio required from Feynman until Galileo is not set.
    MissingCompressionRatio,
    /// The compression ratio is below [`TX_L1_FEE_PRECISION`], i.e. below 1.
    InvalidCompressionRatio(U256),
    /// The compressed size required from Galileo is not set.
    MissingCompressedSize,
    /// The compressed size is larger than the size of the original transaction.
    InvalidCompressedSize {
        /// The size of the compressed rlp-encoded transaction.
        compressed_size: usize,
        /// The size of the original rlp-encoded transaction.
        tx_size: usize,
    },
}

impl core::fmt::Display for L1FeeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingOracleValue => write!(f, "missing L1 gas price oracle value"),
            Self::MissingCompressionRatio => write!(f, "missing transaction compression ratio"),
            Self::InvalidCompressionRatio(ratio) => write!(
                f,
                "transaction compression ratio {ratio} must be greater or equal to {TX_L1_FEE_PRECISION}"
            ),
            Self::MissingCompressedSize => write!(f, "missing transaction compressed size"),
            Self::InvalidCompressedSize { compressed_size, tx_size } => write!(
                f,
                "transaction compressed size {compressed_size} must be less than or equal to the original size {tx_size}"
            ),
        }
    }
}

impl core::error::Error for L1FeeError {}

// L1 BLOCK INFO
// ================================================================================================

/// A struct that holds the L1 block information.
///
/// This struct is used to calculate the gas cost of a transaction based on L1 block data posted on
/// L2.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct L1BlockInfo {
    /// The current L1 base fee.
    pub l1_base_fee: U256,
    /// The current L1 fee overhead.
    pub l1_fee_overhead: U256,
    /// The current L1 fee scalar.
    pub l1_base_fee_scalar: U256,
    /// The current L1 blob base fee, None if before Curie.
    pub l1_blob_base_fee: Option<U256>,
    /// The current L1 commit scalar, None if before Curie.
    pub l1_commit_scalar: Option<U256>,
    /// The current L1 blob scalar, None if before Curie.
    pub l1_blob_scalar: Option<U256>,
    /// The current calldata fee component (l1_commit_scalar * l1_base_fee), None if before Curie.
    ///
    /// Despite the name, this is priced in wei and scaled by [`TX_L1_FEE_PRECISION`], not L1 gas.
    pub calldata_gas: Option<U256>,
    /// The current compression penalty threshold, None if before Feynman.
    pub penalty_threshold: Option<U256>,
    /// The current compression penalty factor, None if before Feynman.
    pub penalty_factor: Option<U256>,
}

impl L1BlockInfo {
    /// Try to fetch the L1 block info from the database.
    pub fn try_fetch<DB: Database>(
        db: &mut DB,
        spec_id: ScrollSpecId,
    ) -> Result<L1BlockInfo, DB::Error> {
        Self::try_fetch_with(|slot| db.storage(L1_GAS_PRICE_ORACLE_ADDRESS, slot), spec_id)
    }

    /// Try to fetch the L1 block info through the journal, so that oracle updates made by earlier
    /// transactions and not yet committed to the database are seen.
    ///
    /// Slots that are not in the journal are read from the database. Unlike `sload`, this does not
    /// load the oracle account or its slots into the journal, so their warm/cold status is
    /// unchanged.
    pub fn try_fetch_from_journal<J: JournalTr + JournalExt>(
        journal: &mut J,
        spec_id: ScrollSpecId,
    ) -> Result<L1BlockInfo, <J::Database as Database>::Error> {
        Self::try_fetch_with(
            |slot| {
                let journaled = journal
                    .evm_state()
                    .get(&L1_GAS_PRICE_ORACLE_ADDRESS)
                    .and_then(|account| account.storage.get(&slot))
                    .map(|slot| slot.present_value);
                match journaled {
                    Some(value) => Ok(value),
                    None => journal.db_mut().storage(L1_GAS_PRICE_ORACLE_ADDRESS, slot),
                }
            },
            spec_id,
        )
    }

    /// Fetches the L1 block info, reading each L1 gas price oracle slot with `storage`.
    fn try_fetch_with<E>(
        mut storage: impl FnMut(U256) -> Result<U256, E>,
        spec_id: ScrollSpecId,
    ) -> Result<L1BlockInfo, E> {
        let l1_base_fee = storage(L1_BASE_FEE_SLOT)?;
        let l1_fee_overhead = storage(L1_OVERHEAD_SLOT)?;
        let l1_base_fee_scalar = storage(L1_SCALAR_SLOT)?;

        // If Curie is not enabled, return the L1 block info without Curie fields.
        if !spec_id.is_enabled_in(ScrollSpecId::CURIE) {
            return Ok(L1BlockInfo {
                l1_base_fee,
                l1_fee_overhead,
                l1_base_fee_scalar,
                ..Default::default()
            });
        }

        let l1_blob_base_fee = storage(L1_BLOB_BASE_FEE_SLOT)?;
        let l1_commit_scalar = storage(L1_COMMIT_SCALAR_SLOT)?;
        let l1_blob_scalar = storage(L1_BLOB_SCALAR_SLOT)?;
        let calldata_gas = l1_commit_scalar.saturating_mul(l1_base_fee);

        // If Feynman is not enabled, return the L1 block info without Feynman fields.
        if !spec_id.is_enabled_in(ScrollSpecId::FEYNMAN) {
            return Ok(L1BlockInfo {
                l1_base_fee,
                l1_fee_overhead,
                l1_base_fee_scalar,
                l1_blob_base_fee: Some(l1_blob_base_fee),
                l1_commit_scalar: Some(l1_commit_scalar),
                l1_blob_scalar: Some(l1_blob_scalar),
                calldata_gas: Some(calldata_gas),
                ..Default::default()
            });
        }

        let penalty_threshold = storage(PENALTY_THRESHOLD_SLOT)?;
        let penalty_factor = storage(PENALTY_FACTOR_SLOT)?;

        Ok(L1BlockInfo {
            l1_base_fee,
            l1_fee_overhead,
            l1_base_fee_scalar,
            l1_blob_base_fee: Some(l1_blob_base_fee),
            l1_commit_scalar: Some(l1_commit_scalar),
            l1_blob_scalar: Some(l1_blob_scalar),
            calldata_gas: Some(calldata_gas),
            penalty_threshold: Some(penalty_threshold),
            penalty_factor: Some(penalty_factor),
        })
    }

    /// Returns the pre-Curie L1 gas for posting the transaction as calldata on L1. Calldata costs
    /// 16 gas per non-zero byte and 4 gas per zero byte, plus the fee overhead and the commit
    /// extra cost.
    ///
    /// The result is in L1 gas, not wei. The caller prices it with the L1 base fee.
    fn pre_curie_l1_gas(&self, input: &[u8]) -> U256 {
        U256::from(input.iter().fold(0, |acc, byte| {
            acc + if *byte == 0x00 { ZERO_BYTE_COST } else { NON_ZERO_BYTE_COST }
        }))
        .saturating_add(self.l1_fee_overhead)
        .saturating_add(TX_L1_COMMIT_EXTRA_COST)
    }

    /// Returns the Curie blob fee component, `blobScalar * _data.length * l1BlobBaseFee`.
    ///
    /// The result is already priced in wei and scaled by [`TX_L1_FEE_PRECISION`], like
    /// `calldata_gas`.
    fn curie_blob_fee(&self, input: &[u8]) -> Result<U256, L1FeeError> {
        let l1_blob_base_fee = self.l1_blob_base_fee.ok_or(L1FeeError::MissingOracleValue)?;
        let l1_blob_scalar = self.l1_blob_scalar.ok_or(L1FeeError::MissingOracleValue)?;
        Ok(U256::from(input.len()).saturating_mul(l1_blob_base_fee).saturating_mul(l1_blob_scalar))
    }

    fn calculate_tx_l1_cost_shanghai(&self, input: &[u8]) -> U256 {
        let tx_l1_gas = self.pre_curie_l1_gas(input);
        tx_l1_gas
            .saturating_mul(self.l1_base_fee)
            .saturating_mul(self.l1_base_fee_scalar)
            .wrapping_div(TX_L1_FEE_PRECISION_U256)
    }

    fn calculate_tx_l1_cost_curie(&self, input: &[u8]) -> Result<U256, L1FeeError> {
        // "commitScalar * l1BaseFee + blobScalar * _data.length * l1BlobBaseFee"
        let blob_fee = self.curie_blob_fee(input)?;
        let calldata_gas = self.calldata_gas.ok_or(L1FeeError::MissingOracleValue)?;

        Ok(calldata_gas.saturating_add(blob_fee).wrapping_div(TX_L1_FEE_PRECISION_U256))
    }

    fn calculate_tx_l1_cost_feynman(
        &self,
        input: &[u8],
        compression_ratio: U256,
    ) -> Result<U256, L1FeeError> {
        // rollup_fee(tx) = size(tx) * (component_exec + component_blob) * penalty(tx)
        //
        // - size(tx): denotes the size of the signed tx.
        //
        // - component_exec: The component that accounts towards committing this tx as part of a L2
        // batch as well as gas costs for the eventual on-chain proof verification.
        //
        // => component_exec = exec_scalar * l1_base_fee
        //    where exec_scalar = compression_scalar * (commit_scalar + verification_scalar)
        //
        // - component_blob: The component that accounts the costs associated with data
        // availability, i.e. the costs of posting this tx's data in the EIP-4844 blob.
        //
        // => component_blob = compressed_blob_scalar * l1_blob_base_fee
        //    where compressed_blob_scalar = compression_scalar * blob_scalar
        //
        // Note that the same slots for L1_COMMIT_SCALAR_SLOT and L1_BLOB_SCALAR_SLOT are
        // re-used for the new exec_scalar and compressed_blob_scalar values post-FEYNMAN.
        //
        // - penalty(tx): A compression penalty based on the transaction's compression ratio.
        //
        // => penalty(tx) = compression_ratio(tx) >= penalty_threshold ? 1 : penalty_factor
        //    where compression_ratio(tx) = size(tx) / size(zstd(tx))
        //
        // Note that commit_scalar (exec_scalar), blob_scalar (compressed_blob_scalar),
        // compression_ratio, penalty_threshold, penalty_factor, penalty are all scaled
        // by TX_L1_FEE_PRECISION_U256 (1e9) to avoid losing precision.

        if compression_ratio < TX_L1_FEE_PRECISION_U256 {
            return Err(L1FeeError::InvalidCompressionRatio(compression_ratio));
        }

        let exec_scalar = self.l1_commit_scalar.ok_or(L1FeeError::MissingOracleValue)?;
        let compressed_blob_scalar = self.l1_blob_scalar.ok_or(L1FeeError::MissingOracleValue)?;
        let l1_blob_base_fee = self.l1_blob_base_fee.ok_or(L1FeeError::MissingOracleValue)?;
        let penalty_threshold = self.penalty_threshold.ok_or(L1FeeError::MissingOracleValue)?;
        let penalty_factor = self.penalty_factor.ok_or(L1FeeError::MissingOracleValue)?;

        let tx_size = U256::from(input.len());

        let component_exec = exec_scalar.saturating_mul(self.l1_base_fee);
        let component_blob = compressed_blob_scalar.saturating_mul(l1_blob_base_fee);
        let fee_per_byte = component_exec.saturating_add(component_blob);

        let penalty = if compression_ratio >= penalty_threshold {
            TX_L1_FEE_PRECISION_U256
        } else {
            penalty_factor
        };

        Ok(tx_size
            .saturating_mul(fee_per_byte)
            .saturating_mul(penalty)
            .wrapping_div(TX_L1_FEE_PRECISION_U256) // account for scalars
            .wrapping_div(TX_L1_FEE_PRECISION_U256)) // account for penalty
    }

    fn calculate_tx_l1_cost_galileo(
        &self,
        tx_size: usize,         // size of the original rlp-encoded transaction
        compressed_size: usize, // size of the compressed rlp-encoded transaction
    ) -> Result<U256, L1FeeError> {
        // Post Galileo rollup fee formula:
        // rollup_fee(tx) = fee_per_byte * compressed_size(tx) * (1 + penalty(tx)) / PRECISION
        //
        // Where:
        // fee_per_byte = (exec_scalar * l1_base_fee + blob_scalar * l1_blob_base_fee)
        // compressed_size(tx) = min(len(zstd(rlp(tx))), len(rlp(tx)))
        // penalty(tx) = compressed_size(tx) / penalty_factor

        if compressed_size > tx_size {
            return Err(L1FeeError::InvalidCompressedSize { compressed_size, tx_size });
        }

        let compressed_size = U256::from(compressed_size);

        let exec_scalar = self.l1_commit_scalar.ok_or(L1FeeError::MissingOracleValue)?;
        let blob_scalar = self.l1_blob_scalar.ok_or(L1FeeError::MissingOracleValue)?;
        let l1_blob_base_fee = self.l1_blob_base_fee.ok_or(L1FeeError::MissingOracleValue)?;

        let penalty_factor = match self.penalty_factor {
            Some(f) if f == U256::ZERO => U256::ONE, // sanitize zero penalty factor
            Some(f) => f,
            None => return Err(L1FeeError::MissingOracleValue),
        };

        // fee_per_byte = (exec_scalar * l1_base_fee) + (blob_scalar * l1_blob_base_fee)
        let component_exec = exec_scalar.saturating_mul(self.l1_base_fee);
        let component_blob = blob_scalar.saturating_mul(l1_blob_base_fee);
        let fee_per_byte = component_exec.saturating_add(component_blob);

        // base_term = fee_per_byte * compressed_size
        let base_term = fee_per_byte.saturating_mul(compressed_size);

        // penalty_term = (base_term * compressed_size) / penalty_factor
        let penalty_term = base_term.saturating_mul(compressed_size).wrapping_div(penalty_factor);

        // rollup_fee = (base_term + penalty_term) / PRECISION
        Ok(base_term.saturating_add(penalty_term).wrapping_div(TX_L1_FEE_PRECISION_U256))
    }

    /// Calculate the gas cost of a transaction based on L1 block data posted on L2.
    ///
    /// Returns an error if an input required by the spec's L1 fee formula is missing or invalid.
    pub fn try_calculate_tx_l1_cost(
        &self,
        input: &[u8],
        spec_id: ScrollSpecId,
        compression_ratio: Option<U256>,
        compressed_size: Option<usize>,
    ) -> Result<U256, L1FeeError> {
        let l1_cost = if !spec_id.is_enabled_in(ScrollSpecId::CURIE) {
            self.calculate_tx_l1_cost_shanghai(input)
        } else if !spec_id.is_enabled_in(ScrollSpecId::FEYNMAN) {
            self.calculate_tx_l1_cost_curie(input)?
        } else if !spec_id.is_enabled_in(ScrollSpecId::GALILEO) {
            let compression_ratio = compression_ratio.ok_or(L1FeeError::MissingCompressionRatio)?;
            self.calculate_tx_l1_cost_feynman(input, compression_ratio)?
        } else {
            let compressed_size = compressed_size.ok_or(L1FeeError::MissingCompressedSize)?;
            self.calculate_tx_l1_cost_galileo(input.len(), compressed_size)?
        };

        if spec_id.is_enabled_in(ScrollSpecId::TSUKI) {
            Ok(l1_cost.min(U96_MAX))
        } else {
            Ok(l1_cost.min(U64_MAX))
        }
    }

    /// Calculate the gas cost of a transaction based on L1 block data posted on L2.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::try_calculate_tx_l1_cost`] returns an error.
    pub fn calculate_tx_l1_cost(
        &self,
        input: &[u8],
        spec_id: ScrollSpecId,
        compression_ratio: Option<U256>,
        compressed_size: Option<usize>,
    ) -> U256 {
        self.try_calculate_tx_l1_cost(input, spec_id, compression_ratio, compressed_size)
            .unwrap_or_else(|err| panic!("{err} in spec_id={spec_id:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::primitives::uint;
    use rstest::rstest;

    #[rstest]
    #[case(50, uint!(171557471810_U256))] // ~0.06 cents
    #[case(100, uint!(344821983141_U256))] // ~0.12 cents
    #[case(1024, uint!(3854009072433_U256))] // ~1.35 cents
    #[case(10 * 1024, uint!(70759382824796_U256))] // ~24.77 cents
    #[case(1024 * 1024, uint!(378961881717079120_U256))] // ~1325 USD
    fn test_rollup_fee_galileo(#[case] compressed_size: usize, #[case] expected: U256) {
        let gpo = L1BlockInfo {
            l1_base_fee: uint!(1_000_000_000_U256),            // 1 gwei
            l1_blob_base_fee: Some(uint!(1_000_000_000_U256)), // 1 gwei
            l1_commit_scalar: Some(uint!(2394981796_U256)),    // 2.39
            l1_blob_scalar: Some(uint!(1019097245_U256)),      // 1.02
            penalty_factor: Some(uint!(10000_U256)),
            ..Default::default()
        };

        // Use a dummy size larger than the compressed size.
        let tx_size = 1e10 as usize;
        let actual = gpo.calculate_tx_l1_cost_galileo(tx_size, compressed_size);
        assert_eq!(Ok(expected), actual);
    }

    #[test]
    fn test_invalid_inputs_return_errors() {
        let gpo = L1BlockInfo {
            l1_blob_base_fee: Some(U256::ONE),
            l1_commit_scalar: Some(U256::ONE),
            l1_blob_scalar: Some(U256::ONE),
            calldata_gas: Some(U256::ONE),
            penalty_threshold: Some(U256::ONE),
            penalty_factor: Some(U256::ONE),
            ..Default::default()
        };
        let input = [1u8; 4];
        let ratio = Some(TX_L1_FEE_PRECISION_U256);

        assert!(gpo.try_calculate_tx_l1_cost(&input, ScrollSpecId::FEYNMAN, ratio, None).is_ok());
        assert!(gpo
            .try_calculate_tx_l1_cost(&input, ScrollSpecId::TSUKI_V2, None, Some(4))
            .is_ok());

        // Oracle values that Curie and later formulas need are not set.
        for spec in [ScrollSpecId::CURIE, ScrollSpecId::FEYNMAN, ScrollSpecId::TSUKI_V2] {
            assert_eq!(
                L1BlockInfo::default().try_calculate_tx_l1_cost(&input, spec, ratio, Some(4)),
                Err(L1FeeError::MissingOracleValue),
                "{spec:?}"
            );
        }

        // Feynman until Galileo needs a compression ratio of at least 1.
        assert_eq!(
            gpo.try_calculate_tx_l1_cost(&input, ScrollSpecId::FEYNMAN, None, Some(4)),
            Err(L1FeeError::MissingCompressionRatio)
        );
        let low_ratio = TX_L1_FEE_PRECISION_U256 - U256::ONE;
        assert_eq!(
            gpo.try_calculate_tx_l1_cost(&input, ScrollSpecId::FEYNMAN, Some(low_ratio), None),
            Err(L1FeeError::InvalidCompressionRatio(low_ratio))
        );

        // Galileo and later need a compressed size no larger than the transaction.
        assert_eq!(
            gpo.try_calculate_tx_l1_cost(&input, ScrollSpecId::TSUKI_V2, ratio, None),
            Err(L1FeeError::MissingCompressedSize)
        );
        assert_eq!(
            gpo.try_calculate_tx_l1_cost(&input, ScrollSpecId::TSUKI_V2, None, Some(5)),
            Err(L1FeeError::InvalidCompressedSize { compressed_size: 5, tx_size: 4 })
        );
    }

    #[test]
    fn test_u96_limbs() {
        let u96_max = U256::from((1u128 << 96) - 1);
        assert_eq!(U96_MAX, u96_max);
    }
}
