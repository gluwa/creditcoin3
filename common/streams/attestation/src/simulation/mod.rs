mod core;

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]
    #[test]
    /// Simulates 1000 pseudo-random testing scenarios for the attestation stream to pass.
    /// Historical fail cases stored inside `proptest-regressions` are also run to avoid
    /// regressions.
    fn simulate(sim in core::simulation("ws://localhost:9944".parse().unwrap())) {
        sim.run()
    }
}
