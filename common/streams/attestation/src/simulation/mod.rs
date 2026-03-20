mod core;

use proptest::prelude::*;

proptest! {
    #[test]
    fn simulate(sim in core::simulation("ws://localhost:9944".parse().unwrap())) {
        sim.run()
    }
}
