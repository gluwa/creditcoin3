//! Runtime benchmarks for `pallet-attest-coin-rewards`.

use super::*;
use frame_benchmarking::v1::benchmarks;
use frame_system::RawOrigin;
use sp_core::H160;

benchmarks! {
    set_attest_coin_token {
        let token = H160::repeat_byte(0x42);
    }: _(RawOrigin::Root, token)
}
