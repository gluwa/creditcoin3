mod core;

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]
    #[test]
    fn simulate(sim in core::simulation("ws://localhost:9944".parse().unwrap())) {
        sim.run()
    }
}
