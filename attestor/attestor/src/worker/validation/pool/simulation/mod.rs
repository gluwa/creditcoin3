use crate::prelude::*;

mod core;

#[derive(Clone, Debug)]
struct SimulationState {
    votes: Vec<common::types::Attestation>,
    attestors: std::collections::BTreeSet<attestor_primitives::AttestorId>,
}

impl proptest_state_machine::ReferenceStateMachine for SimulationState {
    type State = Self;
    type Transition = SimulationStep;

    fn init_state() -> proptest::prelude::BoxedStrategy<Self::State> {
        todo!()
    }

    fn transitions(state: &Self::State) -> proptest::prelude::BoxedStrategy<Self::Transition> {
        todo!()
    }

    fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool {
        todo!()
    }

    fn apply(state: Self::State, transition: &Self::Transition) -> Self::State {
        todo!()
    }
}

#[derive(Clone, Debug)]
enum SimulationStep {
    Pop,
    Push(Result<Valid, Invalid>),
    Event,
}

#[derive(Clone, Debug)]
enum Valid {
    After(common::types::Attestation),
    Before(common::types::Attestation),
}

#[derive(Clone, Debug)]
enum Invalid {
    Duplicate(common::types::Attestation),
    Attestor(attestor_primitives::AttestorId),
    Digest,
    Equivocation,
}

#[derive(Clone, Debug)]
enum Event {
    Finalize(common::types::Height),
    Interval(std::num::NonZero<common::types::Height>),
    Revert(common::types::Height),
    Attestor(Election),
    TargetSampleSize(std::num::NonZeroUsize),
}

#[derive(Clone, Debug)]
enum Election {
    Add(Vec<attestor_primitives::AttestorId>),
    Remove(Vec<attestor_primitives::AttestorId>),
}
