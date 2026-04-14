use crate::prelude::*;
use proptest::prelude::*;

struct SimulationState {
    votes: Vec<common::types::Attestation>,
    attestors: Vec<cc_client::attestor::Attestor>,
    attestor_next: usize,

    attestation_prev: Option<stream::util::AttestationInfo>,

    config: crate::worker::validation::pool::Config,
}

impl SimulationState {}

#[derive(Clone, Debug)]
enum SimulationStep {
    Pop,
    Push(Result<Valid, Invalid>),
    Event,
}

#[derive(Clone, Debug)]
enum Valid {
    Before(common::types::Attestation),
    After(common::types::Attestation),
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

impl SimulationStep {
    pub fn new(state: &SimulationState) -> BoxedStrategy<Self> {
        prop_oneof![
            Just(Self::Pop),
            Valid::new(state).prop_map(|valid| Self::Push(Ok(valid))),
        ]
        .boxed()
    }
}

impl Valid {
    pub fn new(state: &SimulationState) -> BoxedStrategy<Self> {
        todo!()
    }
}

impl Event {
    pub fn new(state: &SimulationState) -> BoxedStrategy<Self> {
        todo!()
    }
}
