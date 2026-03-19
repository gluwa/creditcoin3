use super::*;
use proptest::prelude::*;

pub struct Simulation {
    sut: StreamAttestation,
    steps: Vec<SimulationStep>,

    permit_roots: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    permit_tip: futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,

    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    attestation_next: attestor_primitives::Height,
}

impl std::fmt::Debug for Simulation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulation")
            .field("steps", &self.steps)
            .field("permit_roots", &self.permit_roots)
            .field("permit_tip", &self.permit_tip)
            .field("attestation_interval", &self.attestation_interval)
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
        attestation_prev in 0..1_000u64,
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

        let (permit_roots, stream_roots) = mock::Roots::new(start_height);
        let (permit_tip, stream_tip) = mock::Tip::new(start_height);

        let attestation_interval = std::num::NonZero::new(attestation_interval).unwrap();
        let attestation_prev = stream_util::AttestationInfo {
            height: attestation_prev,
            ..Default::default()
        };
        let attestation_next =
            attestation_interval.get() * (attestation_prev.height / attestation_interval.get() + 1);

        let max_catchup = std::num::NonZero::new(max_catchup).unwrap();

        let config = ConfigBuilder::new()
            .with_cc3(client_cc3)
            .with_chain_key(2u64)
            .with_bls_key(bls_key)
            .with_stream_roots(stream_roots.boxed())
            .with_stream_tip(stream_tip.boxed())
            .with_attestation_interval(attestation_interval)
            .with_attestation_prev(attestation_prev)
            .with_max_catchup(max_catchup)
            .build();

        let stream_attestation = StreamAttestation::new(config);

        Simulation {
            sut: stream_attestation,
            steps,
            permit_roots,
            permit_tip,
            attestation_interval,
            attestation_next
        }
    }
}

#[derive(Debug, Clone)]
enum SimulationStep {
    Root(std::task::Poll<()>),
    Tip(std::task::Poll<()>),
    Finalized(Finalized),
}

#[derive(Debug, Clone)]
enum Finalized {
    Before(attestor_primitives::Height),
    After(attestor_primitives::Height),
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
