use proptest::prelude::*;

pub struct Simulation {
    sut: crate::StreamAttestation,
    steps: Vec<SimulationStep>,

    permit_roots: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    permit_tip: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,

    start_height: attestor_primitives::Height,
    attestation_prev: attestor_primitives::Height,
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    attestation_next: attestor_primitives::Height,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
}

impl std::fmt::Debug for Simulation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulation")
            .field("steps", &self.steps)
            .field("start_height", &self.start_height)
            .field("attestation_prev", &self.attestation_prev)
            .field("attestation_interval", &self.attestation_interval)
            .field("attestation_next", &self.attestation_next)
            .field("max_catchup", &self.max_catchup)
            .finish()
    }
}

impl Simulation {
    pub fn run(mut self) {
        use futures::SinkExt as _;
        use futures::StreamExt as _;

        for step in self.steps {
            match step {
                SimulationStep::Root(poll) => {
                    tokio_test::block_on(self.permit_roots.send(poll)).unwrap();
                }
                SimulationStep::Tip(poll) => {
                    tokio_test::block_on(self.permit_tip.send(poll)).unwrap();
                }
                SimulationStep::Finalized(finalized) => {
                    let info = stream_util::AttestationInfo {
                        height: finalized.height(self.attestation_next, self.attestation_interval),
                        ..Default::default()
                    };

                    self.sut.note_attestation_finalization(info);
                }
            }

            match tokio_test::task::spawn(self.sut.next()).poll() {
                std::task::Poll::Ready(Some(Ok(attestation))) => {
                    self.attestation_next = self.attestation_interval.get()
                        * (attestation.header_number() / self.attestation_interval.get());
                }
                std::task::Poll::Ready(Some(Err(err))) => panic!("{err}"),
                std::task::Poll::Ready(None) => panic!("Attestation stream should be infinite"),
                std::task::Poll::Pending => {}
            }
        }
    }
}

prop_compose! {
    pub fn simulation(
        cc3_url: url::Url,
    )(
        cc3_url in Just(cc3_url),
        attestation_interval in 1..750u64,
        attestation_prev in 0..100u64,
        start_height in 0..1_000u64,
        max_catchup in 1..500u64,
        steps in prop::collection::vec(SimulationStep::step(), 1..1_000)
    ) -> Simulation {
        use futures::StreamExt as _;

        let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
        let bls_key = bls_signatures::PrivateKey::new(secret.to_string().as_bytes());
        let client_cc3 =
            tokio_test::block_on(cc_client::Client::new(cc3_url, &secret.to_string()))
                .expect("Failed to create cc3 client");

        let (permit_roots, stream_roots) = crate::simulation::mock::Roots::new(start_height);
        let (permit_tip, stream_tip) = crate::simulation::mock::Tip::new(start_height);

        let attestation_prev = stream_util::AttestationInfo {
            height: attestation_prev * attestation_interval,
            ..Default::default()
        };
        let attestation_next = attestation_interval * (attestation_prev.height / attestation_interval + 1);
        let attestation_interval = std::num::NonZero::new(attestation_interval).unwrap();

        let max_catchup = std::num::NonZero::new(max_catchup).unwrap();

        let config = crate::ConfigBuilder::new()
            .with_cc3(client_cc3)
            .with_chain_key(2u64)
            .with_bls_key(bls_key)
            .with_stream_roots(stream_roots.boxed())
            .with_stream_tip(stream_tip.boxed())
            .with_attestation_interval(attestation_interval)
            .with_attestation_prev(attestation_prev)
            .with_max_catchup(max_catchup)
            .build();

        let stream_attestation = crate::StreamAttestation::new(config);

        Simulation {
            sut: stream_attestation,
            steps,
            permit_roots,
            permit_tip,

            start_height,
            attestation_prev: attestation_prev.height,
            attestation_interval,
            attestation_next,
            max_catchup,
        }
    }
}

#[derive(Clone)]
enum SimulationStep {
    Root(std::task::Poll<()>),
    Tip(std::task::Poll<()>),
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
            1 => (0..10u64).prop_map(|d| Self::Finalized(Finalized::After(d))),
        ]
    }
}

impl Finalized {
    pub fn height(
        self,
        attestation_next: attestor_primitives::Height,
        attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    ) -> attestor_primitives::Height {
        match self {
            Finalized::Before(delta) => {
                attestation_next.saturating_sub(attestation_interval.get() * delta)
            }
            Finalized::After(delta) => {
                attestation_next.saturating_add(attestation_interval.get() * delta)
            }
        }
    }
}
