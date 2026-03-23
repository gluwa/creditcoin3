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
                    tokio_test::block_on(self.sender_roots.send(poll));
                }
                SimulationStep::Tip(poll) => {
                    // Simulates either a pending stream poll or a ready tip pool
                    tokio_test::block_on(self.sender_tip.send(poll));
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
            }

            // Polls the attestation stream, applying the state transition
            match tokio_test::task::spawn(self.sut.next()).poll() {
                std::task::Poll::Ready(Some(Ok(attestation))) => {
                    self.attestation_prev = attestation.header_number();
                }
                std::task::Poll::Ready(Some(Err(err))) => panic!("{err}"),
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
        use futures::StreamExt as _;

        let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
        let bls_key = bls_signatures::PrivateKey::new(secret.to_string().as_bytes());
        let signer = cc_client::signer::CC3Signer::new(&secret.to_string())
            .expect("Failed to create cc3 signer");

        let (tx_roots, rx_roots) = crate::tests::mock::roots(start_height);
        let (tx_tip, rx_tip) = crate::tests::mock::tip(start_height);

        let attestation_prev = stream_util::AttestationInfo {
            height: start_height,
            ..Default::default()
        };
        let attestation_interval = std::num::NonZero::new(attestation_interval).unwrap();

        let max_catchup = std::num::NonZero::new(max_catchup).unwrap();

        let config = crate::ConfigBuilder::new()
            .with_signer(signer)
            .with_chain_key(2u64)
            .with_bls_key(bls_key)
            .with_stream_roots(rx_roots.boxed())
            .with_stream_tip(rx_tip.boxed())
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
    Finalized(Finalized),
}

#[derive(Clone)]
enum Finalized {
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
            Self::Finalized(finalized) => write!(f, "Finalized({finalized:?})"),
        }
    }
}

impl std::fmt::Debug for Finalized {
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
            2 => Just(Self::Root(std::task::Poll::Pending)),
            2 => Just(Self::Tip(std::task::Poll::Pending)),
            4 => Just(Self::Root(std::task::Poll::Ready(()))),
            4 => Just(Self::Tip(std::task::Poll::Ready(()))),
            1 => (0..10u64).prop_map(|d| Self::Finalized(Finalized::Before(d))),
            1 => (1..10u64).prop_map(|d| Self::Finalized(Finalized::After(d))),
        ]
    }
}

impl Finalized {
    pub fn height(
        self,
        attestation_prev: attestor_primitives::Height,
        attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    ) -> attestor_primitives::Height {
        match self {
            Finalized::Before(delta) => {
                attestation_prev.saturating_sub(attestation_interval.get() * delta)
            }
            Finalized::After(delta) => {
                attestation_prev.saturating_add(attestation_interval.get() * delta)
            }
        }
    }
}
