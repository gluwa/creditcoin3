use anyhow::Result;
use attestation_chain::continuity_chain::CreateResult;
use eth::Client;
use sp_core::H256;
use std::collections::BTreeSet;
use thiserror::Error;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};
use tracing::{debug, error, info, warn};

use attestor_primitives::{Attestation as AttestationPrimitive, AttestorId, ChainKey, Digest};
use cc_client::attestation::Subscription;
use creditcoin3_attestor_gossip::communication::Attestation;

use crate::{cc3, ccsub, eth_sub, fragment, retry, Config};

pub const ATTESTATION_BUFFER_SIZE: usize = 100;

/// Defines how much finalized attestations can be used as a window to check if we already can restart the engine
const ATTESTATIONS_RESTART_WINDOW: u64 = 2;

/// Defines how many epochs the engine can be halted for
const MAX_EPOCHS_HALTED: u64 = 2;

/// Defines how much checkpoints attestations are valid for
pub const ATTESTATION_CHECKPOINT_WINDOW: u64 = 2;

pub struct Engine {
    // Engine state
    state: State,
    // Ethereum client
    eth_client: Client,
    // Creditcoin client
    cc3_client: cc3::Client,
    // Subscription to the source chain
    source_chain_subscription: Option<JoinHandle<()>>,
    // Channels to send / receive attestations from source chain
    sender: Sender<AttestationPrimitive<H256>>,
    receiver: Receiver<AttestationPrimitive<H256>>,
    // Keeps track off all the blocks voted for
    voted_for: BTreeSet<(u64, Digest)>,
    // Keeps track of the last finalized attestation
    last_finalized_attestation_header: u64,
    // Keeps track of the current epoch
    current_epoch: u64,
}

enum State {
    NotRunning,
    Running,
    Stopped,
    // Halted at a certain epoch
    Halted(u64),
}

impl State {
    fn not_running(&self) -> bool {
        !matches!(self, Self::Running)
    }

    fn is_halted(&self) -> bool {
        matches!(self, Self::Halted(_))
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to submit attestation")]
    FailedToSubmit,
    #[error("Double vote")]
    DoubleVote,
    #[error("Engine is not running")]
    NotRunning,
    #[error("Failed to create fragment")]
    FailedToCreateFragment,
    #[error("Cc3 error: {0}")]
    Cc3Error(#[from] cc3::Error),
    #[error("cclient error: {0}")]
    Cclient(#[from] cc_client::Error),
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl Error {
    #[must_use]
    pub fn is_not_selected_error(&self) -> bool {
        matches!(
            self,
            Error::Cclient(cc_client::Error::FailedToCreateProofOfInclusion(
                vrf::Error::NotSelected
            ))
        )
    }

    #[must_use]
    pub fn is_not_running_error(&self) -> bool {
        matches!(self, Error::NotRunning)
    }

    #[must_use]
    pub fn is_double_vote_error(&self) -> bool {
        matches!(self, Error::DoubleVote)
    }
}

impl Engine {
    /// Create a new attestation engine
    /// This will create a new connection to the evm chain and the creditcoin chain
    pub async fn new(config: &Config) -> Result<Self> {
        let eth_client = Client::new(&config.eth_rpc_url, None).await?;
        let chain_id = eth_client.chain_id();
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let cc3_client =
            cc3::Client::new(config.cc3_rpc_url.clone(), &config.cc3_key, chain_id).await?;
        cc3_client.init().await?;

        let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(ATTESTATION_BUFFER_SIZE);

        Ok(Self {
            state: State::NotRunning,
            eth_client,
            cc3_client,
            source_chain_subscription: None,
            sender: attestation_tx,
            receiver: attestation_rx,
            voted_for: BTreeSet::new(),
            last_finalized_attestation_header: 0,
            current_epoch: 0,
        })
    }

    #[must_use]
    fn cc_client(&self) -> &cc3::Client {
        &self.cc3_client
    }

    #[must_use]
    pub fn chain_key(&self) -> ChainKey {
        self.cc3_client.get_chain_key()
    }

    #[must_use]
    fn attestation_interval(&self) -> u64 {
        self.cc3_client.get_attestation_interval()
    }

    pub async fn event_sub(&self) -> Result<Subscription> {
        Ok(self
            .cc3_client
            .inner
            .subscribe_events(self.chain_key())
            .await?)
    }

    pub async fn start(&mut self, mut start_block: u64) -> Result<(), Error> {
        if matches!(self.state, State::Running) {
            return Ok(());
        }

        // If the start block is 0, we need to get the last finalized attestation
        // and start from there
        if start_block == 0 {
            // get last finalized attestation
            let last_finalized = self
                .cc_client()
                .get_last_attestation(self.chain_key())
                .await?;

            if let Some(last_finalized) = last_finalized {
                start_block = last_finalized.header_number() + self.attestation_interval();
            }
        }

        let can_attest = self.cc3_client.can_attest().await?;
        if !can_attest {
            info!("Not allowed to attest in this epoch, waiting until next epoch rotation to reevaluate");
            self.state = State::NotRunning;
            return Ok(());
        }

        info!("Starting attestation engine");
        let attestation_interval = self.cc3_client.get_attestation_interval();

        let cc3_client = self.cc3_client.clone();
        let eth_client = self.eth_client.clone();
        // Safe to clone since it's using an Arc under the hood
        let sender = self.sender.clone();

        // Retrying handle which retries the subscription to new heads
        // If something goes wrong, the function is fired again
        // This function populates the sender channel with new attestations, so when a retry is fired, the queue is not emptied
        // So retrying can cause a lot of duplicate attestations to be sent on the channel, it's up to the consumer to handle this
        self.source_chain_subscription = Some(tokio::task::spawn(async move {
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
        }));

        // Set the state to running
        self.state = State::Running;

        Ok(())
    }

    async fn stop(&mut self) {
        if matches!(self.state, State::NotRunning) {
            return;
        }

        if let Some(task) = self.source_chain_subscription.take() {
            task.abort();
            let _ = task.await; // Await the result to clean up resources properly

            // Recreate the channel
            let (attestation_tx, attestation_rx) =
                tokio::sync::mpsc::channel(ATTESTATION_BUFFER_SIZE);
            self.sender = attestation_tx;
            self.receiver = attestation_rx;

            // Only set the state to stopped if it's not halted
            if !self.state.is_halted() {
                self.state = State::Stopped;
            }
        }
    }

    /// Restart the attestation engine
    /// Will stop the engine if it's running and start it again
    async fn restart(&mut self, start_block: u64) -> Result<(), Error> {
        if matches!(self.state, State::Running) {
            self.stop().await;
        }

        self.start(start_block).await
    }

    /// Poll a new attestation from the source chain
    /// This will block until a new attestation is available
    /// If the engine is not running, it will return None in order to unblock the caller
    /// So preferably if the poll return None, the caller should stop polling or issue a timeout
    pub async fn next(&mut self) -> Option<AttestationPrimitive<H256>> {
        if self.state.not_running() {
            return None;
        }

        debug!("Getting next attestation");
        self.receiver.recv().await
    }

    /// Calculate the number of attestations between checkpoints
    async fn checkpoint_blocks(&self) -> Result<u64> {
        let attestation_interval = self.cc3_client.get_attestation_interval();
        let checkpoint_interval = u64::from(self.cc3_client.get_checkpoint_interval().await?);

        Ok(attestation_interval * checkpoint_interval)
    }

    /// Preare an attestation for submission
    /// This will sign the attestation, if signing fails it means we are not eligible to submit an attestation
    /// A fragment is created for the attestation to prove continuity
    async fn prepare_attestation(
        &self,
        mut attestation: AttestationPrimitive<H256>,
    ) -> Result<Attestation<H256, AttestorId>, Error> {
        let header_number = attestation.header_number;
        let digest = attestation.digest();

        // Exit early if the attestation has already been submitted
        if self.voted_for.contains(&(header_number, digest)) {
            warn!("Attestation already voted for: {}", header_number);
            return Err(Error::DoubleVote);
        }

        // Eligiblity check
        let vrf_output = self.cc3_client.sign_vrf(header_number).await?;

        // Create continuity fragment
        let continuity_fragment = self.create_continuity_proof(header_number).await?;

        // Set the previous digest
        attestation.prev_digest = continuity_fragment.prev_digest;
        let signed_attestation = self.cc3_client.sign_attestation(
            attestation,
            continuity_fragment.continuity_proof,
            vrf_output,
        );

        debug!("Attestor selected for block({})", header_number);

        Ok(signed_attestation)
    }

    /// Submit an attestation to the creditcoin chain
    pub async fn submit_attestation(
        &mut self,
        attestation: AttestationPrimitive<H256>,
    ) -> Result<(), Error> {
        if self.state.not_running() {
            return Err(Error::NotRunning);
        }

        // Prepare the attestation
        let attestation = self.prepare_attestation(attestation).await?;

        // Note voted for the header number
        self.voted_for
            .insert((attestation.header_number(), attestation.digest()));

        info!(
            "Submitted attestation for block({}), digest: {:?}, epoch: {}",
            attestation.header_number(),
            attestation.digest(),
            self.current_epoch
        );

        // Submit the attestation to the chain
        self.cc_client()
            .submit_attestation::<H256>(attestation)
            .await?;

        // Evaluate the voting position
        self.evaluate_voting_position().await?;

        Ok(())
    }

    /// Evaluate the voting position of the attestor
    /// This is important to ensure that the attestor is in line with the chain
    /// If the attestor is ahead of the chain, it will stop the engine and wait for the chain to catch up
    /// If the attestor is behind the chain, it will catch up by polling the next attestations
    async fn evaluate_voting_position(&mut self) -> Result<()> {
        debug!("Evaluating voting position...");

        let last_voted_for_block = self.voted_for.last().copied().unwrap_or_default().0;
        let last_finalized = self.last_finalized_attestation_header;
        info!(
            "Last voted for: {:}, last finalized attestation: {:}",
            last_voted_for_block, last_finalized
        );

        if last_finalized == 0 {
            debug!("No attestations voted for or finalized, skipping evaluation");
            return Ok(());
        }

        let diff = last_voted_for_block.saturating_sub(last_finalized);
        // If the difference is greater than the allowed drift, we need to restart the engine
        let drifted = diff > (self.checkpoint_blocks().await? * ATTESTATION_CHECKPOINT_WINDOW);

        // If we are voting for a block that is behind the last finalized attestation, we need to catch up
        if last_voted_for_block < last_finalized {
            // Drain the engine until we are caught up
            while let Some(attestation) = self.next().await {
                if attestation.header_number >= last_finalized {
                    info!(
                        "Caught up to last finalized attestation: {:}",
                        last_finalized
                    );
                    break;
                }
            }
        } else if drifted {
            warn!("Attestation was finalized, but we are ahead with voting. Last voted for: {:}, last finalized attestation: {:}", last_voted_for_block, last_finalized);
            info!(
                "Stopping the engine at last block voted for: {:}",
                last_voted_for_block
            );

            self.state = State::Halted(self.current_epoch);
            // Stop the engine and allow the chain to catch up
            self.stop().await;
        }

        Ok(())
    }

    pub async fn create_continuity_proof(
        &self,
        attestation_header_number: u64,
    ) -> Result<CreateResult, Error> {
        // We need the last voted digest to create a fragment for the new attestation vote.
        // Otherwise we cannot construct the continuity proof.
        let last_digest_header_number = if let Some(last_voted_for) = self.voted_for.last().copied()
        {
            last_voted_for
        } else {
            // Get last finalized attestation
            let last_attestation = self
                .cc_client()
                .get_last_attestation(self.chain_key())
                .await?;
            if let Some(last_attestation) = last_attestation {
                warn!(
                    "No last voted for attestation, using last finalized attestation from chain: {}",
                    last_attestation.header_number()
                );
                (last_attestation.header_number(), last_attestation.digest())
            } else {
                warn!("No last attestation found, using zero digest");
                (0, H256::zero())
            }
        };

        // Calculate fragment length based on the attestation header number and the last voted for header number
        let fragment_length = attestation_header_number.saturating_sub(last_digest_header_number.0);

        // If fragment length is 0 and we are not attesting for the genesis block,
        // return an error
        if fragment_length == 0 && attestation_header_number != 0 {
            error!(
                "Fragment length is 0, this means we are trying to create a fragment for the same block"
            );
            return Err(Error::FailedToCreateFragment);
        }

        debug!(
            "Creating continuity proof for block({}), last voted for: {}, fragment_length: {}",
            attestation_header_number, last_digest_header_number.0, fragment_length
        );

        // Create the fragment for the signed attestation
        // This is the continuity proof of this signed attestation
        let fragment = fragment::async_retry_create(
            &self.eth_client,
            attestation_header_number,
            fragment_length,
            last_digest_header_number.1,
        )
        .await?;

        debug!(
            "Completed fragment creation for block({})",
            attestation_header_number
        );

        Ok(fragment)
    }

    /// Note a cc event
    /// This is used to handle events from the creditcoin chain
    pub async fn note_cc_event(&mut self, event: ccsub::Event) -> Result<(), Error> {
        match event {
            ccsub::Event::AttestationIntervalChanged((_chain_key, interval)) => {
                self.note_interval_change(interval).await?;
            }
            ccsub::Event::BlockAttested(attestation) => {
                self.note_last_attested_header(attestation.header_number())
                    .await?;
            }
            ccsub::Event::RandomnessChanged((epoch, _randomness)) => {
                self.note_epoch_change(epoch).await?;
            }
        }

        Ok(())
    }

    /// Note the interval change
    /// If the interval changes, we need to restart the engine
    /// Otherwise we do nothing
    async fn note_interval_change(&mut self, new_interval: u64) -> Result<(), Error> {
        let needs_restart = self.cc3_client.get_attestation_interval() != new_interval;
        if needs_restart {
            self.cc3_client.change_attestation_interval(new_interval);
            let start_block = self.voted_for.last().copied().unwrap_or_default().0;
            self.restart(start_block).await?;
        }

        Ok(())
    }

    /// Note the last attested header
    /// We keep track of the last attested header to check if we can start the engine again
    async fn note_last_attested_header(&mut self, header: u64) -> Result<(), Error> {
        self.last_finalized_attestation_header = header;

        let last_voted_for = self.voted_for.last().copied().unwrap_or_default().0;

        info!(
            "Last finalized attestation: {:}, last voted for: {:}",
            header, last_voted_for
        );

        // Check if we can start again
        // By subtracting the restart window from the last voted for block
        if header
            >= last_voted_for
                .saturating_sub(ATTESTATIONS_RESTART_WINDOW * self.attestation_interval())
            && self.state.is_halted()
        {
            info!(
                "Chain caught up, resuming attestation engine at block: {:}",
                last_voted_for
            );
            let start_at = last_voted_for + self.attestation_interval();
            self.start(start_at).await?;
        }

        // Prune old last voted for state
        self.voted_for
            .retain(|(header_number, _)| *header_number > header);

        Ok(())
    }

    /// Note the epoch change
    /// If the epoch changes and we are not running, we need to reevaluate the engine
    async fn note_epoch_change(&mut self, epoch: u64) -> Result<(), Error> {
        info!("Noting current epoch: {}", epoch);
        self.current_epoch = epoch;

        // Grab the last finalized attestation
        let last_finalized_attestation = self
            .cc3_client
            .get_last_attestation(self.chain_key())
            .await?;
        // If there is a last finalized attestation, we can start the engine from there
        if let Some(last_finalized_attestation) = last_finalized_attestation {
            self.last_finalized_attestation_header = last_finalized_attestation.header_number();
        }

        let mut start_at = self.last_finalized_attestation_header;
        // If we don't attest to the genesis block, we need to start at the next interval
        if self.last_finalized_attestation_header != 0 {
            start_at += self.attestation_interval();
        }

        match self.state {
            State::Running => return Ok(()),
            State::Halted(halted_at) => {
                // If we exceed the maximum epochs halted, we need to restart the engine
                if epoch >= halted_at + MAX_EPOCHS_HALTED {
                    info!("Engine is halted, but enough epochs have passed, restarting");
                    // Clear the voted for list
                    self.voted_for.clear();
                    // Start the engine again
                    self.start(start_at).await?;
                    return Ok(());
                }
                info!("Engine is halted, but not enough epochs have passed, waiting");
            }
            // In case we are not running or stopped, we need to start the engine
            _ => {
                self.start(start_at).await?;
            }
        }

        Ok(())
    }
}

/// Subscribes to new Ethereum heads and starts the attestation process.
async fn subscribe_to_new_heads_task(
    cc3_client: cc3::Client,
    eth_client: eth::Client,
    sender: Sender<AttestationPrimitive<H256>>,
    attestation_interval: u64,
    start_block: u64,
) -> Result<()> {
    let chain_key = cc3_client.get_chain_key();

    // Calculate the target header to subscribe to
    // Which is the start_header (last finalized attestation) + the checkpoint interval X attestation interval because we want to limit
    // going to the next checkpoint
    // So in essence, we are subscribing for block between two checkpoints
    let checkpoint_interval = cc3_client.get_checkpoint_interval().await?;
    let target_header = start_block + (u64::from(checkpoint_interval) * attestation_interval);

    Ok(eth_sub::attest_to_heads(
        eth_client,
        sender,
        start_block,
        target_header,
        chain_key,
        attestation_interval,
    )
    .await?)
}
