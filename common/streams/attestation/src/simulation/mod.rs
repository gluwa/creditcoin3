mod core;
mod mock;

pub use mock::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn simulate(sim in core::simulation("ws://localhost:9944".parse().unwrap())) {
        sim.run()
    }
}
