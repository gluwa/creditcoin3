use super::*;
use crate::Runtime;
use pallet_ethereum::FinalizationWeight;

#[test]
fn runtime_admission_charges_assembly_without_lending_it_to_evm_execution() {
    use ethereum::{legacy::TransactionSignature, LegacyTransaction, TransactionAction};
    use frame_support::{
        dispatch::{DispatchClass, GetDispatchInfo},
        traits::Hooks,
    };
    use pallet_evm::GasWeightMapping;
    use sp_core::{H160, H256, U256};
    use sp_runtime::BuildStorage;

    let storage = frame_system::GenesisConfig::<Runtime>::default()
        .build_storage()
        .unwrap();
    sp_io::TestExternalities::new(storage).execute_with(|| {
        assert!(<Runtime as pallet_evm::Config>::config().max_refund_quotient >= 5);
        let transaction = pallet_ethereum::Transaction::Legacy(LegacyTransaction {
            nonce: U256::zero(),
            gas_price: U256::one(),
            gas_limit: 21_000.into(),
            action: TransactionAction::Call(H160::repeat_byte(1)),
            value: U256::zero(),
            input: vec![],
            signature: TransactionSignature::new(38, H256::repeat_byte(1), H256::repeat_byte(2))
                .unwrap(),
        });
        let bare = <Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(21_000, true);
        let runner = crate::Ethereum::transaction_weight(&(&transaction).into())
            .0
            .unwrap();
        assert_eq!(runner, bare);
        let call = pallet_ethereum::Call::<Runtime>::transact { transaction };
        let info = call.get_dispatch_info();
        assert_eq!(
            info.call_weight,
            bare.saturating_add(WeightInfo::<Runtime>::transaction(21_000))
        );
        assert!(crate::Ethereum::on_initialize(1).all_gte(WeightInfo::<Runtime>::base()));

        let limits = crate::BlockWeights::get();
        let normal = limits.get(DispatchClass::Normal);
        let fill = normal
            .max_total
            .unwrap()
            .saturating_sub(bare)
            .saturating_sub(normal.base_extrinsic);
        crate::System::register_extra_weight_unchecked(fill, DispatchClass::Normal);
        assert_eq!(
            call.pre_dispatch_self_contained(&H160::zero(), &info, 0),
            Some(Err(
                sp_runtime::transaction_validity::InvalidTransaction::ExhaustsResources.into()
            ))
        );
        assert_eq!(pallet_ethereum::Pending::<Runtime>::count(), 0);
    });
}

#[test]
fn gas_reservations_cover_the_committed_benchmark_dimensions() {
    use single_component::WeightInfo as Bench;
    let root = Bench::<Runtime>::on_finalize_state_root(15_000)
        .saturating_sub(<Runtime as frame_system::Config>::DbWeight::get().writes(15_000));
    let cases = [
        (
            Bench::<Runtime>::on_finalize_transactions(3_571),
            3_571 * 21_000,
        ),
        (
            Bench::<Runtime>::on_finalize_logs(197_000),
            21_000 + 197_000 * 375,
        ),
        (
            Bench::<Runtime>::on_finalize_topics(40_000),
            21_000 + 40_000 * 1_875,
        ),
        (
            Bench::<Runtime>::on_finalize_log_data(8_700),
            21_000 + 8_700 * 8_567,
        ),
        (
            Bench::<Runtime>::on_finalize_transaction_size(3_840),
            21_000 + 3_840 * 4_096,
        ),
        (root, 21_000 + 15_000 * 2_000),
    ];
    for (measured, gas_before_refund) in cases {
        let gas = gas_before_refund * 4 / 5;
        let reserved =
            WeightInfo::<Runtime>::base().saturating_add(WeightInfo::<Runtime>::transaction(gas));
        assert!(
            reserved.all_gte(measured),
            "{reserved:?} does not cover {measured:?}"
        );
    }
}

#[test]
fn gas_envelope_also_covers_the_combined_measurement() {
    let measured = full_block::WeightInfo::<Runtime>::on_finalize_full_block(
        3_600, 40_000, 9_000, 5_120, 15_000,
    )
    .saturating_sub(<Runtime as frame_system::Config>::DbWeight::get().writes(15_000));
    // The synthetic combined fixture spends several blocks' worth of gas. It is a useful
    // interaction check, but its sum of independent maxima must not set a per-block cap.
    let gas =
        (3_600_u64 * 21_000 + 40_000 * 1_875 + 9_000 * 8_567 + 5_120 * 4_096 + 15_000 * 2_000) * 4
            / 5;
    let reserved =
        WeightInfo::<Runtime>::base().saturating_add(WeightInfo::<Runtime>::transaction(gas));
    assert!(reserved.all_gte(measured));
}

#[test]
fn reservation_is_monotonic_and_subadditive_so_refunds_cannot_erase_it() {
    let gas = [0, 1, 21_000, 100_000, 75_000_000];
    for pair in gas.windows(2) {
        assert!(WeightInfo::<Runtime>::transaction(pair[1])
            .all_gte(WeightInfo::<Runtime>::transaction(pair[0])));
    }
    for a in gas {
        for b in gas {
            assert!(WeightInfo::<Runtime>::transaction(a)
                .saturating_add(WeightInfo::<Runtime>::transaction(b))
                .all_gte(WeightInfo::<Runtime>::transaction(a + b)));
        }
    }
}
