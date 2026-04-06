use proptest::prelude::*;

/// Simulation configuration, drives the state transitions being applied to the
/// [attestation stream].
///
/// [attestation stream]: crate::StreamAttestation
pub struct Simulation {
    sut: crate::StreamAttestation,
    /// Collection of state transitions to be applied to the attestation stream as part of a single
    /// simulation run.
    steps: Vec<SimulationStep>,

    /// Mocked source chain [roots stream] with control over stream yielding.
    ///
    /// [roots stream]: stream_eth::StreamRoots
    sender_roots: crate::tests::mock::RootSender,
    /// Mocked source chain [tip stream] with control over stream yielding.
    ///
    /// [tip stream]: stream_eth::StreamTip
    sender_tip: crate::tests::mock::TipSender,

    attestor: cc_client::attestor::Attestor,

    start_height: attestor_primitives::Height,
    attestation_prev: attestor_primitives::Height,
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
}

impl std::fmt::Debug for Simulation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulation")
            .field("steps", &self.steps)
            .field("start_height", &self.start_height)
            .field("attestation_prev", &self.attestation_prev)
            .field("attestation_interval", &self.attestation_interval)
            .field("max_catchup", &self.max_catchup)
            .finish()
    }
}

impl Simulation {
    /// Runs the simulated [`steps`] on the attestation stream to completion.
    ///
    /// No invariants are checked inside this function. Instead, we rely on the assertions inside
    /// the attestation stream to detect any invalid state.
    ///
    /// [`steps`]: Self::steps
    pub fn run(mut self) {
        use futures::StreamExt as _;

        for step in self.steps {
            match step {
                SimulationStep::Root(poll) => {
                    // Simulates either a pending stream poll or a ready root pool
                    self.sender_roots.send(poll);
                }
                SimulationStep::Tip(poll) => {
                    // Simulates either a pending stream poll or a ready tip pool
                    self.sender_tip.send(poll);
                }
                SimulationStep::Finalized(finalized) => {
                    // Simulates an attestation finalizing either before or after the previous
                    // attestation
                    let info = stream_util::AttestationInfo {
                        height: finalized.height(self.attestation_prev, self.attestation_interval),
                        ..Default::default()
                    };

                    self.sut.note_attestation_finalization(info);
                }
                SimulationStep::IntervalChange(interval_new) => {
                    // Simulates a change in the attestation interval
                    tokio_test::block_on(self.sut.note_attestation_interval_change(interval_new));

                    self.attestation_interval = interval_new;
                }
                SimulationStep::ChainReversion(reversion) => {
                    // Simulates a chain reversion either before or after the previous attestation.
                    let info = stream_util::AttestationInfo {
                        height: reversion.height(self.attestation_prev, self.attestation_interval),
                        ..Default::default()
                    };

                    tokio_test::block_on(self.sut.note_attestation_chain_reversion(info));
                }
            }

            // Polls the attestation stream, applying the state transition
            match tokio_test::task::spawn(self.sut.next()).poll() {
                std::task::Poll::Ready(Some(permit)) => {
                    self.attestation_prev = self
                        .sut
                        .generate_attestation(&self.attestor, permit)
                        .header_number();
                }
                std::task::Poll::Ready(None) => panic!("Attestation stream should be infinite"),
                std::task::Poll::Pending => {}
            }
        }
    }
}

prop_compose! {
    /// See the [Proptest Book] for more information on input generation and shrinking.
    ///
    /// [Proptest Book]: https://proptest-rs.github.io/proptest/intro.html
    pub fn simulation()(
        attestation_interval in 1..750u64,
        start_height in 0..100u64,
        max_catchup in 1..500u64,
        steps in prop::collection::vec(SimulationStep::step(), 1..1_000)
    ) -> Simulation {
        use stream_util::ChainExt as _;

        let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
        let attestor = cc_client::attestor::Attestor::new(secret.into(), 2)
            .expect("Failed to initialize attestor");

        let (tx_roots, rx_roots) = crate::tests::mock::roots(start_height);
        let (tx_tip, rx_tip) = crate::tests::mock::tip(start_height);

        let attestation_prev = stream_util::AttestationInfo {
            height: start_height,
            ..Default::default()
        };
        let attestation_interval = std::num::NonZero::new(attestation_interval).unwrap();

        let max_catchup = std::num::NonZero::new(max_catchup).unwrap();

        let config = crate::ConfigBuilder::new()
            .with_chain_key(2u64)
            .with_stream_roots(rx_roots.boxed_data())
            .with_stream_tip(rx_tip.boxed_data())
            .with_attestation_interval(attestation_interval)
            .with_attestation_prev(attestation_prev)
            .with_max_catchup(max_catchup)
            .build();

        let stream_attestation = crate::StreamAttestation::new(config);

        Simulation {
            sut: stream_attestation,
            steps,
            sender_roots: tx_roots,
            sender_tip: tx_tip,
            attestor,

            start_height,
            attestation_prev: attestation_prev.height,
            attestation_interval,
            max_catchup,
        }
    }
}

/// State transitions to be applied to the attestation stream during a [`Simulation`] run.
#[derive(Clone)]
enum SimulationStep {
    /// Sends data to the [root stream].
    ///
    /// [root stream]: stream_eth::StreamRoots
    Root(std::task::Poll<()>),
    /// Sends data to the [tip stream].
    ///
    /// [tip stream]: stream_eth::StreamTip
    Tip(std::task::Poll<()>),
    /// Notifies the [attestation stream] of a new finalized attestation. This can either be a past
    /// attestation, in which case it should be ignored, or a future attestation, in which case it
    /// should update the root cache.
    ///
    /// [attestation stream]: crate::StreamAttestation
    Finalized(ChainPoint),
    /// Notifies the [attestation stream] of a change in the attestation interval. This is always
    /// guaranteed to be non-zero as it is assumed the validation step should take place further up
    /// in the call stack.
    ///
    /// [attestation stream]: crate::StreamAttestation
    IntervalChange(std::num::NonZero<attestor_primitives::Height>),
    /// Notifies the [attestation stream] of a chain reversion. This can point to either a past or
    /// future attestation. Contrary to attestation finalization, chain reversions should always
    /// be applied even if the attestation being reverted to is locally unknown up to that point.
    ///
    /// [attestation stream]: crate::StreamAttestation
    ChainReversion(ChainPoint),
}

#[derive(Clone)]
enum ChainPoint {
    Before(attestor_primitives::Height),
    After(attestor_primitives::Height),
}

impl std::fmt::Debug for SimulationStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root(std::task::Poll::Pending) => write!(f, "Root(Pending)"),
            Self::Root(std::task::Poll::Ready(())) => write!(f, "Root(Ready)"),
            Self::Tip(std::task::Poll::Pending) => write!(f, "Tip(Pending)"),
            Self::Tip(std::task::Poll::Ready(())) => write!(f, "Tip(Ready)"),
            Self::Finalized(point) => write!(f, "Finalized({point:?})"),
            Self::IntervalChange(interval) => write!(f, "IntervalChange({interval})"),
            Self::ChainReversion(point) => write!(f, "ChainReversion({point:?})"),
        }
    }
}

impl std::fmt::Debug for ChainPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Before(delta) => write!(f, "Before({delta})"),
            Self::After(delta) => write!(f, "After({delta})"),
        }
    }
}

impl SimulationStep {
    pub fn step() -> impl Strategy<Value = Self> {
        prop_oneof![
            4 => Just(Self::Root(std::task::Poll::Pending)),
            4 => Just(Self::Tip(std::task::Poll::Pending)),
            6 => Just(Self::Root(std::task::Poll::Ready(()))),
            6 => Just(Self::Tip(std::task::Poll::Ready(()))),
            1 => (0..10u64).prop_map(|d| Self::Finalized(ChainPoint::Before(d))),
            1 => (1..10u64).prop_map(|d| Self::Finalized(ChainPoint::After(d))),
            2 => (1..100u64).prop_map(|n| Self::IntervalChange(std::num::NonZero::new(n).unwrap())),
            1 => (0..10u64).prop_map(|d| Self::ChainReversion(ChainPoint::Before(d))),
            1 => (1..10u64).prop_map(|d| Self::ChainReversion(ChainPoint::After(d))),
        ]
    }
}

impl ChainPoint {
    pub fn height(
        self,
        attestation_prev: attestor_primitives::Height,
        attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    ) -> attestor_primitives::Height {
        match self {
            ChainPoint::Before(delta) => {
                attestation_prev.saturating_sub(attestation_interval.get() * delta)
            }
            ChainPoint::After(delta) => {
                attestation_prev.saturating_add(attestation_interval.get() * delta)
            }
        }
    }
}
