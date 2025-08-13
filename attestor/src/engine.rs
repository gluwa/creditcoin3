use anyhow::Result;
use eth::Client;
use sp_core::H256;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::{
    sync::{
        mpsc::{Receiver, Sender, UnboundedSender},
        Mutex,
    },
    task::JoinHandle,
    time::sleep,
};
use tracing::{debug, error, info, warn};

use attestor_primitives::{Attestation as AttestationPrimitive, AttestorId, ChainKey, Digest};
use cc_client::attestation::Subscription;
use creditcoin3_attestor_gossip::communication::Attestation;

use crate::{
    cc3, ccsub, continuity, error::Error, eth_sub, metric_set_label, prom, sync_state::SyncState,
    Config,
};

use crate::prom::AttestorMetrics;

pub const ATTESTATION_BUFFER_SIZE: usize = 100;

/// Defines how much finalized attestations can be used as a window to check if we already can restart the engine
const ATTESTATIONS_RESTART_WINDOW: u64 = 2;

/// Defines how many epochs the engine can be halted for
const MAX_EPOCHS_HALTED: u64 = 2;

/// Defines how much checkpoints attestations are valid for
pub const ATTESTATION_CHECKPOINT_WINDOW: u64 = 2;

/// Defines how long we will wait before retrying the attestation subscription
pub const ATTESTATION_SUB_DELAY: Duration = Duration::from_secs(5);

/// Defines how many times we will retry the attestation subscription
pub const ATTESTATION_SUB_MAX_RETRIES: u64 = 50;

#[derive(Debug)]
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
    // Keeps track of the current epoch
    current_epoch: u64,
    // Keeps track of the starting block provided by the user
    start_block: u64,
    // Continuity cache
    continuity_cache: continuity::Cache,
    // Sync state
    sync_state: SyncState,
    // Maturity delay
    maturity_delay: u64,
    // Shutdown channel to gracefully stop the engine
    shutdown_channel: UnboundedSender<()>,
    // Prometheus metrics
    metrics: Option<AttestorMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug)]
pub struct AsyncEngine {
    inner: Arc<Mutex<Engine>>,
    eth_client: Arc<Client>,
    pub chain_key: ChainKey,
}

impl Clone for AsyncEngine {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            eth_client: Arc::clone(&self.eth_client),
            chain_key: self.chain_key,
        }
    }
}

impl AsyncEngine {
    pub async fn new(config: &Config, shutdown: UnboundedSender<()>) -> Result<Self> {
        let eth_client = Client::new(&config.eth_rpc_url, None).await?;
        let target = eth_client.get_last_block().await?;
        let chain_id = eth_client.chain_id();

        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let cc3_client = cc3::Client::new(
            config.cc3_rpc_url.clone(),
            &config.cc3_key,
            config.chain_key,
            chain_id,
        )
        .await?;
        cc3_client.init().await?;

        let chain_key = cc3_client.get_chain_key();

        let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(ATTESTATION_BUFFER_SIZE);

        let (_, last_finalized_header) = get_last_finalized(&cc3_client, chain_key)
            .await?
            .unwrap_or_default();

        let start_block = cc3_client
            .get_attestation_chain_genesis_block_number()
            .await?;

        let current_epoch = cc3_client.get_current_epoch().await?;

        // Register metrics server if configured
        let metrics = if config.enable_prometheus_metrics {
            debug!("Starting Prometheus metrics server");
            prom::start_prom_server(config)
        } else {
            None
        };

        let engine: Engine = Engine {
            state: State::NotRunning,
            eth_client: eth_client.clone(),
            cc3_client,
            source_chain_subscription: None,
            sender: attestation_tx,
            receiver: attestation_rx,
            voted_for: BTreeSet::new(),
            sync_state: SyncState::new(last_finalized_header, target),
            current_epoch,
            start_block,
            continuity_cache: continuity::Cache::new(eth_client.clone()),
            maturity_delay: config.maturity_delay,
            shutdown_channel: shutdown,
            metrics,
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(engine)),
            eth_client: Arc::new(eth_client),
            chain_key,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let mut engine = self.inner.lock().await;
        engine.start().await
    }

    pub async fn stop(&mut self) {
        let mut engine = self.inner.lock().await;
        engine.stop().await;
    }

    /// Poll a new attestation from the source chain
    /// This will block until a new attestation is available
    /// If the engine is not running, it will return None in order to unblock the caller
    /// So preferably if the poll return None, the caller should stop polling or issue a timeout
    pub async fn next(&mut self) -> Option<AttestationPrimitive<H256>> {
        let mut engine = self.inner.lock().await;

        if engine.state.not_running() {
            return None;
        }

        let maturity_delay = engine.maturity_delay;
        debug!("Getting next attestation");
        let attestation = engine.receiver.recv().await;
        drop(engine);

        if let Some(attestation) = attestation {
            return Some(
                self.mature_block(attestation, maturity_delay)
                    .await
                    .expect("Failed to mature block"),
            );
        }

        None
    }

    /// Submit an attestation to the creditcoin chain
    pub async fn submit_attestation(
        &mut self,
        attestation: AttestationPrimitive<H256>,
    ) -> Result<(), Error> {
        let mut engine = self.inner.lock().await;

        if engine.state.not_running() {
            return Err(Error::NotRunning);
        }

        // Note voted for the header number
        // Returns a boolean indicating if it already was inserted in the set
        let can_submit = engine
            .voted_for
            .insert((attestation.header_number(), attestation.digest()));
        // Another sanity check to ensure we are not double voting
        if !can_submit {
            info!(
                "❗ Double vote detected for block: {}, digest: {}",
                attestation.header_number(),
                attestation.digest()
            );
            return Err(Error::DoubleVote(attestation.header_number()));
        }

        // Prepare the attestation
        let attestation = engine.prepare_attestation(attestation).await?;
        let digest = attestation.digest();
        let round = attestation.round();

        // Submit the attestation to the chain
        engine
            .cc_client()
            .submit_attestation::<H256>(attestation)
            .await?;

        info!(
            "✉️ Submitted attestation for round: {:?}, digest: {:?}, epoch: {}",
            round, digest, engine.current_epoch
        );

        // Evaluate the voting position
        engine.evaluate_voting_position().await?;

        Ok(())
    }

    /// Note a cc event
    /// This is used to handle events from the creditcoin chain
    pub async fn note_cc_event(&mut self, event: ccsub::Event) -> Result<(), Error> {
        let mut engine = self.inner.lock().await;
        engine.note_cc_event(event).await
    }

    pub async fn event_sub(&self) -> Result<Subscription> {
        let guard = self.inner.lock().await;
        Ok(guard
            .cc3_client
            .inner
            .subscribe_events(self.chain_key)
            .await?)
    }

    async fn mature_block(
        &self,
        attestation: AttestationPrimitive<sp_core::H256>,
        delay: u64,
    ) -> Result<AttestationPrimitive<sp_core::H256>> {
        // Check if we can mature the block
        let check_interval = tokio::time::Duration::from_secs(10);

        loop {
            // Get current eth head first
            let last_eth_block_number = match self.eth_client.get_last_block().await {
                Ok(block_number) => block_number,
                Err(e) => {
                    error!("Failed to get last block number: {:?}", e);
                    return Err(e.into());
                }
            };

            // update source chain height
            let inner = self.inner.lock().await;
            metric_set_label!(
                inner.metrics,
                source_chain_height,
                inner.chain_key(),
                last_eth_block_number
            );
            drop(inner);

            // If the attestation is mature, return it
            if attestation.header_number <= last_eth_block_number - delay {
                return Ok(attestation);
            }

            info!(
                "👶 Attestation not mature, Current block: {}, required block: {}, waiting...",
                last_eth_block_number,
                attestation.header_number + delay
            );

            // Wait for check interval before checking again
            sleep(check_interval).await;
        }
    }
}

impl Engine {
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

    async fn start(&mut self) -> Result<(), Error> {
        if matches!(self.state, State::Running) {
            return Ok(());
        }

        let can_attest = self.cc3_client.can_attest().await?;
        if !can_attest {
            info!("🔴 Not allowed to attest in this epoch, waiting until next epoch rotation to reevaluate");
            self.state = State::NotRunning;
            return Ok(());
        }

        info!("🟢 Starting attestation engine");

        let cc3_client = self.cc3_client.clone();
        let eth_client = self.eth_client.clone();
        // Safe to clone since it's using an Arc under the hood
        let sender = self.sender.clone();
        // Clone the shutdown channel to be able to send a shutdown signal
        let shutdown = self.shutdown_channel.clone();
        // Get the start block from the engine
        let start_block = self.start_block;

        // Retrying handle which retries the subscription to new heads
        // If something goes wrong, the function is fired again
        self.source_chain_subscription = Some(tokio::spawn(async move {
            let mut attempts = 0;

            loop {
                info!(
                    "🔄 Attempting to subscribe to new heads, starting at block: {}",
                    start_block
                );
                let result = subscribe_to_new_heads_task(
                    cc3_client.clone(),
                    eth_client.clone(),
                    sender.clone(),
                    start_block,
                )
                .await;

                match result {
                    Ok(()) => {
                        error!("🔴 Attestation stream exited unexpectedly with Ok()");
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        error!(
                            "🔴 Attestation stream exited unexpectedly with error: {:?}",
                            e
                        );

                        if attempts > ATTESTATION_SUB_MAX_RETRIES {
                            error!("Reached max retry attempts. Giving up.");
                            shutdown.send(()).ok();
                            break;
                        }
                        info!(
                            "Retrying attestation sub in {:?} seconds... ",
                            ATTESTATION_SUB_DELAY,
                        );
                        tokio::time::sleep(ATTESTATION_SUB_DELAY).await;
                    }
                }
            }
        }));

        // Set the state to running
        self.state = State::Running;

        info!("🟢 Attestation engine started at block: {}", start_block);

        Ok(())
    }

    async fn stop(&mut self) {
        info!("🛑 Stopping attestation engine");
        if matches!(self.state, State::NotRunning) {
            return;
        }

        if let Some(task) = self.source_chain_subscription.take() {
            task.abort();
            let _ = task.await; // Await the result to clean up resources properly
            debug!("Attestation engine subscription stopped");

            // Only set the state to stopped if it's not halted
            if !self.state.is_halted() {
                self.state = State::Stopped;
            }
        }
    }

    /// Restart the attestation engine
    /// Will stop the engine if it's running and start it again
    async fn restart(&mut self) -> Result<(), Error> {
        // Stop the engine if it's running
        self.stop().await;

        // Clean up the channels
        self.receiver.close();
        // Drain the channel to ensure no messages are left in the buffer
        while (self.receiver.recv().await).is_some() {
            debug!("Draining attestation channel...");
        }

        // Recreate the channel
        let (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(ATTESTATION_BUFFER_SIZE);
        self.sender = attestation_tx;
        self.receiver = attestation_rx;

        // Reset the state
        self.voted_for.clear();

        debug!("🔄 Restarting attestation engine, drained channel, cleared voted_for and resetted state");

        self.start().await
    }

    /// Poll a new attestation from the source chain
    /// This will block until a new attestation is available
    /// If the engine is not running, it will return None in order to unblock the caller
    /// So preferably if the poll return None, the caller should stop polling or issue a timeout
    async fn next(&mut self) -> Option<AttestationPrimitive<H256>> {
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

    /// Evaluate the voting position of the attestor
    /// This is important to ensure that the attestor is in line with the chain
    /// If the attestor is ahead of the chain, it will stop the engine and wait for the chain to catch up
    /// If the attestor is behind the chain, it will catch up by polling the next attestations
    async fn evaluate_voting_position(&mut self) -> Result<()> {
        debug!("Evaluating voting position...");

        let last_voted_for_block = self.voted_for.last().copied().unwrap_or_default().0;
        let last_finalized = self.sync_state.current();
        debug!(
            "Last voted for: {:}, last finalized attestation: {:}",
            last_voted_for_block, last_finalized
        );
        // set last voted for
        metric_set_label!(
            self.metrics,
            last_voted_for,
            self.chain_key(),
            last_voted_for_block
        );

        // Determine drift baseline
        let baseline = if last_finalized == 0 && self.start_block > 0 {
            self.start_block
        } else {
            last_finalized
        };

        let diff = last_voted_for_block.saturating_sub(baseline);
        let drifted = diff > (self.checkpoint_blocks().await? * ATTESTATION_CHECKPOINT_WINDOW);

        // If we are voting for a block that is behind the last finalized attestation, we need to catch up
        if last_voted_for_block < last_finalized {
            // Drain the engine until we are caught up
            while let Some(attestation) = self.next().await {
                if attestation.header_number >= last_finalized {
                    debug!(
                        "Caught up to last finalized attestation: {:}",
                        last_finalized
                    );
                    break;
                }
            }
        } else if drifted {
            warn!("⚠️ Attestation was finalized, but we are ahead with voting. Last voted for: {:}, last finalized attestation: {:}", last_voted_for_block, last_finalized);
            info!(
                "🛑 Stopping the engine at last block voted for: {:?}, current epoch: {}",
                last_voted_for_block, self.current_epoch
            );

            self.state = State::Halted(self.current_epoch);
        }

        Ok(())
    }

    /// Preare an attestation for submission
    /// This will sign the attestation, if signing fails it means we are not eligible to submit an attestation
    /// A fragment is created for the attestation to prove continuity
    async fn prepare_attestation(
        &mut self,
        mut attestation: AttestationPrimitive<H256>,
    ) -> Result<Attestation<H256, AttestorId>, Error> {
        let header_number = attestation.header_number;

        // Eligiblity check
        let vrf_output = self.cc3_client.sign_vrf(header_number).await.map_err(|e| {
            debug!("Error signing vrf: {:?}", e);
            Error::NotSelected(header_number)
        })?;

        // Create continuity fragment
        let continuity_fragment = self.create_continuity_proof(header_number).await?;

        let current_epoch = self.cc_client().get_current_epoch().await?;

        // Set the previous digest
        attestation.prev_digest = Some(
            continuity_fragment
                .head_digest()
                .map_or_else(sp_core::H256::zero, |d| H256::from(d.to_bytes_be())),
        );
        debug!("Previous digest set, {:?}", attestation.prev_digest);

        // Serialize the fragment to be sent over the wire
        let serialized_fragment =
            continuity::AttestationFragmentSerializable::from(&continuity_fragment);

        let signed_attestation = self.cc3_client.sign_attestation(
            attestation,
            serialized_fragment,
            vrf_output,
            current_epoch,
        );

        debug!("Attestor selected for block({})", header_number);

        Ok(signed_attestation)
    }

    /// Create a continuity proof for the given attestation header number.
    /// It always starts from the last finalized attestation and continues until the given header number.
    pub async fn create_continuity_proof(
        &mut self,
        attestation_header_number: u64,
    ) -> Result<continuity::AttestationFragment, Error> {
        if attestation_header_number == 0 {
            info!("🛠️ Creating default continuity proof for header number 0");
            return Ok(continuity::AttestationFragment::default());
        }

        // Get last attested source chain block number
        let result = get_last_finalized(&self.cc3_client, self.chain_key()).await?;

        // From which point we want to create a continuity proof
        let (from_header, from_digest) = if let Some((digest, header_number)) = result {
            debug!(
                "Last finalized source block found: header_number={}, digest={}",
                header_number, digest
            );
            // No need to include the last finalized attestation inside the continuity proof
            // We should start from the next block and provide the finalized digest to the fragment creation
            (header_number.saturating_add(1), digest)
        } else {
            warn!("No last finalized source block found, starting from configured starting block");
            // Treating provided start block as genesis block
            (self.start_block, H256::zero())
        };

        // Short circuit if we are already at or past the attestation header number
        if from_header > attestation_header_number {
            return Err(Error::AlreadyAttestedTo(attestation_header_number));
        }

        let until_block = if attestation_header_number == from_header {
            // Meaning it's the first attestation in the chain
            attestation_header_number
        } else {
            // We don't need to include the attestation itself inside the continuity proof
            // So we subtract 1 from the attestation header number
            attestation_header_number.saturating_sub(1)
        };

        // Create the fragment for the signed attestation
        // This is the continuity proof of this signed attestation
        let fragment = self
            .continuity_cache
            .async_retry_create(from_header, from_digest, until_block)
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
        match event.clone() {
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
            ccsub::Event::CheckpointReached(ck, checkpoint) => {
                if self.chain_key() != ck {
                    debug!("Ignoring checkpoint for different chain key");
                    return Ok(());
                }

                // Prune the continuity cache
                self.continuity_cache
                    .prune_all_before(checkpoint.block_number);
                // Prune the voted for state
                self.voted_for
                    .retain(|(header_number, _)| *header_number > checkpoint.block_number);
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
            info!(
                "🔀 Attestation interval changed to {}, restarting engine",
                new_interval
            );
            self.restart().await?;
        }

        Ok(())
    }

    /// Note the last attested header
    /// We keep track of the last attested header to check if we can start the engine again
    async fn note_last_attested_header(&mut self, header: u64) -> Result<(), Error> {
        let last_voted_for = self.voted_for.last().copied().unwrap_or_default().0;

        debug!(
            "Last finalized attestation: {:}, last voted for: {:}",
            header, last_voted_for
        );
        // set last finalized attesation
        metric_set_label!(
            self.metrics,
            last_finalized_attestation,
            self.chain_key(),
            header
        );

        // Check if we can start again
        // By subtracting the restart window from the last voted for block
        if header
            >= last_voted_for
                .saturating_sub(ATTESTATIONS_RESTART_WINDOW * self.attestation_interval())
            && self.state.is_halted()
        {
            info!(
                "🟢 Chain caught up, resuming attestation engine at block: {:}",
                last_voted_for
            );
            self.state = State::Running;
        }

        // Update the sync state
        let last_eth_height = self.eth_client.get_last_block().await?;
        self.sync_state.update(header, last_eth_height);

        Ok(())
    }

    /// Note the epoch change
    /// If the epoch changes and we are not running, we need to reevaluate the engine
    async fn note_epoch_change(&mut self, epoch: u64) -> Result<(), Error> {
        debug!("Noting current epoch: {}", epoch);
        self.current_epoch = epoch;

        // set current epoch
        let chain_key = self.chain_key();
        metric_set_label!(self.metrics, cc_current_epoch, chain_key, epoch);

        match self.state {
            State::Running => return Ok(()),
            State::Halted(halted_at) => {
                // If we exceed the maximum epochs halted, we need to restart the engine
                if epoch >= halted_at + MAX_EPOCHS_HALTED {
                    info!("🫱 Engine is halted, but enough epochs have passed, restarting");
                    return self.restart().await;
                }
                info!("✋ Engine is halted, but not enough epochs have passed, waiting");
            }
            // In case we are not running or stopped, we need to start the engine
            _ => {
                self.start().await?;
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
    start_block: u64,
) -> Result<()> {
    let attestation_interval = cc3_client.get_attestation_interval();
    let chain_key = cc3_client.get_chain_key();

    // If the start block is 0, we need to get the last attested source block
    // and start from there
    let result = get_last_finalized(&cc3_client, chain_key).await?;
    let start = if let Some((_, last_finalized)) = result {
        last_finalized + attestation_interval
    } else {
        start_block
    };

    // Calculate the target header to subscribe to
    // Which is the start_header (last finalized attestation) + the checkpoint interval X attestation interval because we want to limit
    // going to the next checkpoint
    // So in essence, we are subscribing for block between two checkpoints
    let checkpoint_interval = cc3_client.get_checkpoint_interval().await?;
    let target_header = start_block + (u64::from(checkpoint_interval) * attestation_interval);

    Ok(eth_sub::attest_to_heads(
        eth_client,
        sender,
        start,
        target_header,
        chain_key,
        attestation_interval,
    )
    .await?)
}
// Handles the edge case where we are beginning attestation after calling `import_checkpoints` for a source chain.
// In such a case, there may be a finalized checkpoint even when there are no finalized attestations.
async fn get_last_finalized(
    cc3_client: &cc3::Client,
    chain_key: ChainKey,
) -> Result<Option<(Digest, u64)>, Error> {
    if let Some(last_attestation) = cc3_client.get_last_attestation(chain_key).await? {
        Ok(Some((
            last_attestation.digest(),
            last_attestation.header_number(),
        )))
    } else if let Some(last_checkpoint) = cc3_client.get_last_checkpoint(chain_key).await? {
        Ok(Some((last_checkpoint.digest, last_checkpoint.block_number)))
    } else {
        Ok(None)
    }
}
