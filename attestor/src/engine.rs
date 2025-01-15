use anyhow::Result;
use eth::Client;
use kameo::spawn;
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestorId, ChainKey, SignedAttestation,
};
use cc_client::attestation::Subscription;
use creditcoin3_attestor_gossip::communication::Attestation;

use crate::{attestation, cc3, eth_sub, retry, Config};

/// Defines how much checkpoints attestations are valid for
pub const ATTESTATION_CHECKPOINT_WINDOW: u64 = 2;

pub struct Engine {
    eth_client: Client,
    cc3_client: cc3::Client,
    task: Option<JoinHandle<()>>,
    sender: Sender<Option<AttestationPrimitive<H256>>>,
    receiver: Receiver<Option<AttestationPrimitive<H256>>>,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to submit attestation")]
    FailedToSubmit,
    #[error("Cc3 error: {0}")]
    Cc3Error(#[from] cc3::Error),
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl Engine {
    /// Create a new attestation engine
    /// This will create a new connection to the evm chain and the creditcoin chain
    pub async fn new(config: &Config) -> Result<Self> {
        let eth_client = Client::new(&config.eth_rpc_url, &String::new()).await?;
        let chain_id = eth_client.chain_id();
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let cc3_client =
            cc3::Client::new(config.cc3_rpc_url.clone(), &config.cc3_key, chain_id).await?;
        cc3_client.init().await?;

        let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(1);

        Ok(Self {
            eth_client,
            cc3_client,
            task: None,
            sender: attestation_tx,
            receiver: attestation_rx,
        })
    }

    fn is_running(&self) -> bool {
        self.task.is_some()
    }

    #[must_use]
    pub fn cc_client(&self) -> &cc3::Client {
        &self.cc3_client
    }

    #[must_use]
    pub fn eth_client(&self) -> &Client {
        &self.eth_client
    }

    #[must_use]
    pub fn chain_key(&self) -> ChainKey {
        self.cc3_client.get_chain_key()
    }

    pub async fn event_sub(&self) -> Result<Subscription> {
        let chain_key = self.chain_key();
        Ok(self
            .cc3_client
            .cc_client
            .subscribe_events(chain_key)
            .await?)
    }

    async fn start(&mut self, start_block: Option<u64>) -> Result<(), Error> {
        let can_attest = self.cc3_client.can_attest().await?;

        if !can_attest {
            info!("Not allowed to attest in this epoch, waiting until next epoch rotation to reevaluate");
            return Ok(());
        }

        info!("Starting attestation engine");
        let attestation_interval = self.cc3_client.get_attestation_interval();

        let cc3_client = self.cc3_client.clone();
        let eth_client = self.eth_client.clone();
        let sender = self.sender.clone();

        let handle = tokio::task::spawn(async move {
            match retry::ret(
                || async {
                    let cc3_client = cc3_client.clone();
                    let eth_client = eth_client.clone();
                    let sender = sender.clone();

                    subscribe_to_new_heads_task(
                        cc3_client,
                        eth_client,
                        sender,
                        attestation_interval,
                        start_block,
                    )
                    .await
                },
                10,
                10,
                None,
            )
            .await
            {
                Ok(()) => info!("Attestation engine stopped"),
                Err(e) => error!("Attestation engine stopped with error: {:?}", e),
            }
        });

        self.task = Some(handle);

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await; // Await the result to clean up resources properly

            // Recreate the channel
            let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(1);
            self.sender = attestation_tx;
            self.receiver = attestation_rx;
        }
    }

    /// Evaluate the attestation engine
    /// This will stop the engine if needed and / or start it again
    pub async fn evaluate(&mut self, start_block: Option<u64>) -> Result<(), Error> {
        let can_attest = self.cc3_client.can_attest().await?;
        if !can_attest {
            if self.is_running() {
                warn!("Not allowed to attest, stopping attestation engine...");
                self.stop().await;
            } else {
                info!("Not allowed to attest in this epoch, waiting until next epoch rotation to reevaluate");
            }
            return Ok(());
        }

        // Only start the engine if it is not running
        if !self.is_running() {
            // Start
            self.start(start_block).await?;
        }

        Ok(())
    }

    pub fn change_interval(&mut self, new_interval: u64) {
        self.cc3_client.change_attestation_interval(new_interval);
    }

    pub async fn next(&mut self) -> Option<AttestationPrimitive<H256>> {
        self.receiver.recv().await.unwrap()
    }

    /// Calculate the number of attestations between checkpoints
    async fn checkpoint_blocks(&self) -> Result<u64> {
        let attestation_interval = self.cc3_client.get_attestation_interval();
        let checkpoint_interval = u64::from(self.cc3_client.get_checkpoint_interval().await?);

        Ok(attestation_interval * checkpoint_interval)
    }

    async fn last_finalized_attestation(
        &self,
    ) -> Result<Option<SignedAttestation<H256, cc_client::AccountId32>>> {
        Ok(self
            .cc3_client
            .get_last_attestation(self.chain_key())
            .await?)
    }

    /// Submit an attestation to the creditcoin chain
    /// This will check if the attestation is within bounds and submit it
    /// If the attestation is too far ahead, the engine will be stopped and restarted
    pub async fn submit_attestation(
        &mut self,
        attestation: Attestation<H256, AttestorId>,
    ) -> Result<(), Error> {
        if let Some(last_attestation) = self.last_finalized_attestation().await?.as_ref() {
            // Calculate 2 checkpoint blocks
            let checkpoint_blocks = self.checkpoint_blocks().await? * ATTESTATION_CHECKPOINT_WINDOW;
            let last_header = last_attestation.header_number();

            if attestation.header_number() >= last_header + checkpoint_blocks {
                warn!("Attestation is too far ahead, restarting engine to be more in line with what the nodes expect");
                // Stop and restart the engine
                self.stop().await;
                self.evaluate(Some(last_header)).await?;

                return Err(Error::FailedToSubmit);
            }
        };

        info!("Attestation is within bounds, submitting");
        self.cc_client()
            .submit_attestation::<H256>(attestation.clone())
            .await?;

        Ok(())
    }
}

/// Subscribes to new Ethereum heads and starts the attestation process.
async fn subscribe_to_new_heads_task(
    cc3_client: cc3::Client,
    eth_client: eth::Client,
    sender: Sender<Option<AttestationPrimitive<H256>>>,
    attestation_interval: u64,
    start_block: Option<u64>,
) -> Result<()> {
    let attestor = spawn(attestation::Attestor::default());
    let chain_key = cc3_client.get_chain_key();

    let start_header = if let Some(start_block) = start_block {
        debug!(
            "Starting from block: {}",
            start_block + attestation_interval
        );
        start_block + attestation_interval
    } else if let Some(last_attestation) = cc3_client.get_last_attestation(chain_key).await? {
        debug!(
            "Last finalized attestation digest: {}",
            last_attestation.digest()
        );
        last_attestation.header_number() + attestation_interval
    } else {
        debug!("No last attestation found, starting from 0");
        0
    };

    // Calculate the target header to subscribe to
    // Which is the start_header (last finalized attestation) + the checkpoint interval X attestation interval because we want to limit
    // going to the next checkpoint
    // So in essence, we are subscribing for block between two checkpoints
    let checkpoint_interval = cc3_client.get_checkpoint_interval().await?;
    let target_header = start_header + (u64::from(checkpoint_interval) * attestation_interval);

    let eth_client_clone = eth_client.clone();
    let sender = sender.clone();
    let attestor = attestor.clone();

    Ok(eth_sub::attest_to_heads(
        eth_client_clone,
        attestor,
        sender,
        start_header,
        target_header,
        chain_key,
        attestation_interval,
    )
    .await?)
}
