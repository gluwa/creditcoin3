use anyhow::Result;
use eth::Client;
use kameo::spawn;
use sp_core::H256;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{debug, info};

use attestor_primitives::{Attestation as AttestationPrimitive, ChainKey};
use cc_client::attestation::Subscription;

use crate::{attestation, cc3, eth_sub};

pub struct Engine {
    eth_client: Client,
    cc3_client: cc3::Client,
    task: Option<JoinHandle<()>>,
    sender: Sender<Option<AttestationPrimitive<H256>>>,
    receiver: Receiver<Option<AttestationPrimitive<H256>>>,
}

impl Engine {
    #[must_use]
    pub fn new(eth_client: Client, cc3_client: cc3::Client) -> Self {
        let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(1);

        Self {
            eth_client,
            cc3_client,
            task: None,
            sender: attestation_tx,
            receiver: attestation_rx,
        }
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

    pub fn event_sub(&self) -> Result<Subscription> {
        let chain_key = self.chain_key();
        Ok(self.cc3_client.cc_client.subscribe_events(chain_key)?)
    }

    pub async fn start(&mut self) -> Result<()> {
        let can_attest = self.cc3_client.can_attest().await?;

        if !can_attest {
            info!("Not allowed to attest, skipping attestation engine");
            return Ok(());
        }

        info!("Starting attestation engine");
        let attestation_interval = self.cc3_client.get_attestation_interval();

        let handle = subscribe_to_new_heads_task(
            self.eth_client.clone(),
            &self.cc3_client,
            self.sender.clone(),
            attestation_interval,
        )
        .await?;

        self.task = Some(handle);

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    /// Evaluate the attestation engine
    /// This will stop the engine and start it again
    pub async fn evaluate(&mut self) -> Result<()> {
        // Stop
        self.stop();

        // Start
        self.start().await?;

        Ok(())
    }

    pub fn change_interval(&mut self, new_interval: u64) {
        self.cc3_client.change_attestation_interval(new_interval);
    }

    pub async fn next(&mut self) -> Option<AttestationPrimitive<H256>> {
        self.receiver.recv().await.unwrap()
    }
}

/// Subscribes to new Ethereum heads and starts the attestation process.
async fn subscribe_to_new_heads_task(
    eth_client: Client,
    cc3_client: &cc3::Client,
    sender: Sender<Option<AttestationPrimitive<H256>>>,
    attestation_interval: u64,
) -> Result<JoinHandle<()>> {
    let attestor = spawn(attestation::Attestor::default());
    let chain_key = cc3_client.get_chain_key();

    let last_attestation = cc3_client.get_last_attestation(chain_key).await?;

    let start_header = if let Some(last_attestation) = last_attestation {
        info!(
            "Last finalized attestation digest: {}",
            last_attestation.digest()
        );
        last_attestation.header_number() + attestation_interval
    } else {
        info!("No last attestation found, starting from 0");
        0
    };

    // Calculate the target header to subscribe to
    // Which is the start_header (last finalized attestation) + the checkpoint interval X attestation interval because we want to limit
    // going to the next checkpoint
    // So in essence, we are subscribing for block between two checkpoints
    let checkpoint_interval = cc3_client.get_checkpoint_interval().await?;
    let target_header = start_header + (u64::from(checkpoint_interval) * attestation_interval);

    Ok(tokio::spawn(async move {
        info!(
            "Subscribing to new heads at target block: {}",
            target_header
        );
        if let Err(e) = eth_sub::attest_to_heads(
            eth_client,
            attestor,
            sender,
            start_header,
            target_header,
            chain_key,
            attestation_interval,
        )
        .await
        {
            debug!("Error in subscribing to new heads: {:?}", e);
        }
    }))
}
