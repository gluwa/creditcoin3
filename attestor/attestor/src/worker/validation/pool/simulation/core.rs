use crate::prelude::*;
use proptest::prelude::*;

#[derive(Clone, Debug)]
struct SimulationState {
    votes: Vec<common::types::Attestation>,
    attestors: Vec<super::mock::Attestor>,
    attestor_next: usize,

    attestation_next: common::types::Height,
    attestation_prev: Option<common::types::AttestationInfo>,
    attestation_interval: std::num::NonZero<common::types::Height>,

    config: crate::worker::validation::pool::Config,
}

impl proptest_state_machine::ReferenceStateMachine for SimulationState {
    type State = Self;
    type Transition = SimulationStep;

    fn init_state() -> BoxedStrategy<Self::State> {
        (
            (0..100usize),
            (0..100u64),
            (1..100usize),
            proptest::option::of(0..100u64),
            (1..100u64),
        )
            .prop_map(
                |(max_size, attestors, quorum, attestation_start, attestation_interval)| Self {
                    votes: vec![],
                    attestors: Self::attestors(attestors).collect(),
                    attestor_next: 0,
                    attestation_next: attestation_start.unwrap_or_default(),
                    attestation_prev: Self::attestation_start(attestation_start),
                    attestation_interval: std::num::NonZero::new(attestation_interval).unwrap(),
                    config: crate::worker::validation::pool::ConfigBuilder::new()
                        .with_max_size(std::num::NonZero::new(max_size).unwrap())
                        .with_attestors(
                            Self::attestors(attestors)
                                .map(|att| att.id())
                                .collect::<Vec<_>>(),
                        )
                        .with_quorum(std::num::NonZero::new(quorum).unwrap())
                        .with_attestation_start(Self::attestation_start(attestation_start))
                        .build(),
                },
            )
            .boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        todo!()
    }

    fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool {
        todo!()
    }

    fn apply(state: Self::State, transition: &Self::Transition) -> Self::State {
        todo!()
    }
}

impl SimulationState {
    fn attestors(n: attestor_primitives::Height) -> impl Iterator<Item = super::mock::Attestor> {
        (0..n).map(super::mock::Attestor::new)
    }

    fn attestation_start(
        n: Option<attestor_primitives::Height>,
    ) -> Option<common::types::AttestationInfo> {
        n.map(|height| common::types::AttestationInfo {
            height,
            ..Default::default()
        })
    }

    fn attestation(&self, height: attestor_primitives::Height) -> common::types::Attestation {
        assert!(self.attestation_prev.is_some());
        assert!(self.attestor_next < self.attestors.len());

        let start = self.attestation_prev.unwrap_or_default();
        assert!(start.height < height);

        let cap = (start.height - height - 1) as usize;
        let blocks = ((start.height + 1)..height).fold(
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(cap),
            |mut acc, h| {
                let digest_prev = acc.last().map(|block| block.digest).unwrap_or(start.digest);

                let block = attestor_primitives::block::Block::new_from_prev_digest(
                    h,
                    sp_core::H256(std::array::from_fn(|n| ((u8::MAX << n) as u64 & h) as u8)),
                    digest_prev,
                );

                acc.push(block.into());
                acc
            },
        );

        let continuity_proof =
            attestor_primitives::attestation_fragment::AttestationFragmentSerializable { blocks };

        let attestation_data = attestor_primitives::AttestationData::new(
            2,
            height,
            sp_core::H256(std::array::from_fn(|n| {
                ((u8::MAX << n) as u64 & height) as u8
            })),
            sp_core::H256(std::array::from_fn(|n| {
                ((u8::MAX << n) as u64 & height) as u8
            })),
            continuity_proof.head().map(|head| head.digest),
        );

        self.attestors
            .iter()
            .nth(self.attestor_next)
            .unwrap()
            .sign_attestation(attestation_data, continuity_proof)
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
