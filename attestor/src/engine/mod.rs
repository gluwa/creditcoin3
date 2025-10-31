// --- std ---
use std::collections::{BTreeMap, BTreeSet};

// --- external crates ---
use anyhow::Result;
use sp_core::H256;
use tokio::{
    sync::{
        broadcast,
        mpsc::{self, Receiver},
    },
    task::JoinHandle,
    time::Instant,
};
use tracing::{debug, error, info, warn};

// --- workspace crates ---
use attestor_primitives::{AttestorId, ChainKey, Digest};
use cc_client::attestation::{CcEvent, Subscription};
use ccnext_abi_encoding::abi::EncodingVersion;
use creditcoin3_attestor_gossip::communication::Attestation;
use eth::{subscription::Height, Client};

// --- local crate ---
use crate::{
    cc3, continuity, error::Error, metric_set_labels, prom, prom::AttestorMetrics, Config,
};

mod constants;
mod retry;
mod source_chain;
mod sync_state;

use constants::{ATTESTATIONS_RESTART_WINDOW, ATTESTATION_BUFFER_SIZE};
use retry::BackoffNext;
use sync_state::SyncState;

/// Public handle to interact with the attestor service.
pub struct AttestorHandle {
    cmd_tx: mpsc::Sender<Command>,
    attestation_rx: broadcast::Receiver<Attestation<H256, AttestorId>>,
}

impl AttestorHandle {
    pub async fn shutdown(&self) -> Result<()> {
        self.cmd_tx
            .send(Command::Shutdown)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    pub fn subscribe_attestations(&self) -> broadcast::Receiver<Attestation<H256, AttestorId>> {
        self.attestation_rx.resubscribe()
    }
}

// Commands sent into or from the service actor.
enum Command {
    Shutdown,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    NotRunning,
    Running,
    Stopped,
    /// New: exponential backoff pause that retries until finality catches up or a wall-clock cap hits.
    PausedBackoff {
        since: std::time::Instant,
        attempt: u32,
        total_paused: std::time::Duration,
        reason: &'static str,
    },
}

impl State {
    fn is_paused_backoff(&self) -> bool {
        matches!(self, State::PausedBackoff { .. })
    }

    fn is_running(&self) -> bool {
        matches!(self, State::Running)
    }
}

/// Single-task service that owns the state machine and IO.
pub struct AttestorService {
    // Engine state
    state: State,
    // Ethereum client
    eth_client: Client,
    // Creditcoin client
    cc3_client: cc3::Client,
    // Attestations outbound
    attestation_tx: broadcast::Sender<Attestation<H256, AttestorId>>,
    // Incoming commands
    cmd_rx: mpsc::Receiver<Command>,
    cmd_tx: mpsc::Sender<Command>,
    // Keeps track of all the blocks voted for (dedupe)
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
    // Prometheus metrics
    metrics: Option<AttestorMetrics>,
    // Chain key
    chain_key: ChainKey,

    // Source-chain subscription pump
    source_chain_task: Option<JoinHandle<()>>,
    // Source-chain heads receiver (unmatured)
    heads_rx: Option<Receiver<Height>>,

    // CC events
    cc_events: Option<Subscription>,

    // Backoff timer
    next_backoff_deadline: Option<Instant>,

    // Maturity queue
    pending: BTreeMap<u64, Vec<u64>>,

    chain_name: String,

    // Block encoding
    encoding: EncodingVersion,
}

impl AttestorService {
    pub async fn spawn(config: &Config) -> Result<AttestorHandle> {
        // Clients
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
        let encoding = EncodingVersion::from(cc3_client.get_chain_encoding());

        let (attestation_tx, attestation_rx) = broadcast::channel(ATTESTATION_BUFFER_SIZE);

        // Get last finalized header from CC chain (or checkpoint)
        let (_, last_finalized_header) = get_last_finalized(&cc3_client, chain_key)
            .await?
            .unwrap_or_default();

        let start_block = cc3_client
            .get_attestation_chain_genesis_block_number()
            .await?;

        let current_epoch = cc3_client.get_current_epoch().await?;

        let chain_name = cc3_client
            .inner
            .get_chain_name()
            .await
            .unwrap_or_else(|_| "dev".to_string());

        // Register metrics server if configured
        let metrics = if config.enable_prometheus_metrics {
            let address_str = format!("{}:{}", config.prometheus_host, config.prometheus_port);
            info!(
                "📈 Starting Prometheus metrics server on http://{}/metrics",
                address_str
            );
            prom::start_prom_server(config, &chain_name)
        } else {
            None
        };

        let (cmd_tx, cmd_rx) = mpsc::channel(64);

        let mut svc = AttestorService {
            state: State::NotRunning,
            eth_client: eth_client.clone(),
            cc3_client,
            attestation_tx,
            cmd_rx,
            cmd_tx: cmd_tx.clone(),
            voted_for: BTreeSet::new(),
            sync_state: SyncState::new(last_finalized_header, target),
            current_epoch,
            start_block,
            continuity_cache: continuity::Cache::new(eth_client.clone(), encoding),
            maturity_delay: config.maturity_delay,
            metrics,
            chain_key,
            source_chain_task: None,
            heads_rx: None,
            cc_events: None,
            next_backoff_deadline: None,
            pending: BTreeMap::new(),
            chain_name,
            encoding,
        };

        // Spawn the actor
        tokio::spawn(async move {
            if let Err(e) = svc.run().await {
                error!("AttestorService stopped with error: {:?}", e);
            }
        });

        Ok(AttestorHandle {
            cmd_tx,
            attestation_rx,
        })
    }

    async fn run(&mut self) -> anyhow::Result<()> {
        // Start engine if eligible
        self.start_running().await?;

        loop {
            // Snapshot optionals/flags
            let have_heads = self.heads_rx.is_some();
            let have_cc = self.cc_events.is_some();
            let backoff = self.next_backoff_deadline;

            // Use a never-polled placeholder when there's no backoff scheduled.
            // (Sleep futures register with the timer only on first poll.)
            let backoff_deadline_fallback =
                tokio::time::Instant::now() + std::time::Duration::from_secs(365 * 24 * 60 * 60);
            let sleep_backoff =
                tokio::time::sleep_until(backoff.unwrap_or(backoff_deadline_fallback));

            tokio::select! {
                // -------- Source-chain heads (guarded) --------
                maybe_head = async {
                    self.heads_rx.as_mut().expect("ensured by guard").recv().await
                }, if have_heads => {
                    if let Some(block_number) = maybe_head {
                        self.pending.entry(block_number).or_default().push(block_number);
                        info!(
                            "📥 Buffered block with header number: {}; pending={} items",
                            block_number,
                            self.pending.values().map(Vec::len).sum::<usize>()
                        );

                        if matches!(self.state, State::Running) {
                            self.drain_matured().await?;
                        }
                    }
                }

                // -------- CC3 events (guarded) --------
                evt = async {
                    self.cc_events.as_mut().expect("ensured by guard").next().await
                }, if have_cc => {
                    if let Some(evt) = evt {
                        self.note_cc_event(evt).await?;
                    } else {
                        warn!("Creditcoin event stream ended");
                        break;
                    }
                }

                // -------- Backoff timer (guarded, no unwrap) --------
                () = sleep_backoff, if backoff.is_some() => {
                    self.next_backoff_deadline = None;
                    self.backoff_tick_once_and_reschedule().await?;
                }

                // -------- External/Internal commands --------
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(Command::Shutdown) | None => {
                            self.stop_internal().await;
                            break;
                        },
                        Some(Command::Restart) => {
                            self.restart().await?;
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn start_running(&mut self) -> Result<(), Error> {
        // eligibility
        let can_attest = self.cc3_client.can_attest().await?;
        if !can_attest {
            info!("🔴 Not allowed to attest in this epoch, waiting until next epoch rotation to reevaluate");
            self.state = State::NotRunning;
            // Still start event stream so we get the epoch rotation
            self.attach_cc_events().await?;
            return Ok(());
        }

        info!("🟢 Starting attestation service");

        // make sure there is no old pump/buffer
        if let Some(h) = self.source_chain_task.take() {
            h.abort();
            let _ = h.await;
        }
        self.heads_rx = None;
        self.pending.clear();

        self.attach_cc_events().await?;
        self.attach_source_chain().await?;
        self.state = State::Running;
        Ok(())
    }

    async fn restart(&mut self) -> Result<(), Error> {
        info!("🔄 Restarting attestation service");
        // Clean up last voted for state
        self.voted_for.clear();
        // Reset the backoff state
        self.next_backoff_deadline = None;
        // Reattach subscriptions and start running again
        self.start_running().await
    }

    async fn stop_internal(&mut self) {
        info!("🛑 Stopping attestation service");
        if let Some(h) = self.source_chain_task.take() {
            h.abort();
            let _ = h.await;
        }
        self.cc_events = None;
        self.state = State::Stopped;
    }

    async fn attach_cc_events(&mut self) -> Result<(), Error> {
        if self.cc_events.is_some() {
            info!("🔔 Already subscribed to Creditcoin events, skipping re-subscription");
            return Ok(());
        }

        info!(
            "🔔 Subscribing to Creditcoin events for chain key: {:?}",
            self.chain_key
        );
        let sub = self
            .cc3_client
            .inner
            .subscribe_events(self.chain_key)
            .await?;
        self.cc_events = Some(sub);
        Ok(())
    }

    async fn attach_source_chain(&mut self) -> Result<(), Error> {
        // Determine start & target
        let attestation_interval = self.cc3_client.get_attestation_interval();
        let (start, needs_genesis_attestation) = if let Some((_, last_finalized)) =
            get_last_finalized(&self.cc3_client, self.chain_key).await?
        {
            // If the block we last voted for is bigger than the last finalized it means the protocol is still finalizing
            // the last votes for, it's safe to assume we can start again from the last voted for block otherwise there will
            // potentially be a double vote or a misaligned interval
            let last_voted_for_block = self.voted_for.last().copied().unwrap_or_default().0;
            let actual_start = if last_voted_for_block > last_finalized {
                last_voted_for_block
            } else {
                last_finalized
            };

            // Start at next interval
            (actual_start + attestation_interval, false)
        } else {
            // No finalized attestations yet - this is a bootstrap scenario
            // We need to attest to the genesis block first
            info!(
                "🚀 Bootstrap mode: Will attest to genesis block {}",
                self.start_block
            );
            (self.start_block, true)
        };

        // If this is a bootstrap scenario, immediately attest to the genesis block
        if needs_genesis_attestation {
            info!(
                "🔨 Preparing genesis block attestation for block {}",
                self.start_block
            );

            // Immediately prepare and submit the genesis block attestation
            // No maturity delay needed for the genesis block
            let signed = self.prepare_attestation(self.start_block).await?;
            let digest = signed.digest();
            let round = signed.round();
            self.cc3_client
                .submit_attestation::<H256>(signed.clone())
                .await?;
            self.voted_for.insert((self.start_block, digest));
            // After submitting genesis attestation, enter backoff to wait for finalization
            info!("⏸️ Entering backoff after genesis attestation to wait for finalization");
            self.evaluate_voting_position().await?;
            return Ok(());
        }

        let vote_acceptance_window = self.vote_acceptance_window().await?;
        let target_header = start + vote_acceptance_window;

        // Channel and spawn
        let (tx, rx) = mpsc::channel(ATTESTATION_BUFFER_SIZE);
        let eth = self.eth_client.clone();
        let cmd_tx = self.cmd_tx.clone();
        self.source_chain_task = Some(tokio::spawn(async move {
            if let Err(e) =
                source_chain::attest_to_heads(eth, tx, start, target_header, attestation_interval)
                    .await
            {
                error!("source chain subscription ended: {:?}", e);
                info!("🔄 Requesting attestation service restart");
                // Request restart
                if let Err(send_err) = cmd_tx.send(Command::Restart).await {
                    error!("Failed to send restart command: {:?}", send_err);
                }
            }
        }));
        self.heads_rx = Some(rx);
        info!("🟢 Attestation service subscribed at block: {}", start);
        Ok(())
    }

    // Called on timer or nudged by events
    async fn backoff_tick_once_and_reschedule(&mut self) -> Result<()> {
        match self.backoff_tick_once().await? {
            BackoffNext::Stop => {
                if !self.state.is_running() {
                    // ✅ Finality caught up enough — reattach subscriptions and resume
                    info!("Restarting engine after backoff tick was fired.");
                    self.restart().await?;
                }
            }
            BackoffNext::Continue => {
                if let State::PausedBackoff { attempt, .. } = self.state {
                    let delay = crate::engine::retry::jittered_backoff(attempt);
                    self.next_backoff_deadline = Some(Instant::now() + delay);
                }
            }
        }
        Ok(())
    }

    /// Evaluate the voting position of the attestor and maybe enter backoff.
    async fn evaluate_voting_position(&mut self) -> Result<()> {
        debug!("Evaluating voting position...");

        let last_voted_for_block = self.voted_for.last().copied().unwrap_or_default().0;
        if last_voted_for_block == self.start_block {
            // Pause until we have an event that indicates we attested to "genesis" correctly
            info!("At starting (genesis) block, waiting for attestation events to proceed");
            self.state = State::PausedBackoff {
                since: std::time::Instant::now(),
                attempt: 0,
                total_paused: std::time::Duration::ZERO,
                reason: "at_genesis_block",
            };

            info!(
                "⏸️ Pausing (backoff) at genesis block; epoch={}",
                self.current_epoch
            );
            self.next_backoff_deadline = Some(Instant::now() + std::time::Duration::from_secs(12));

            return Ok(());
        }

        let last_finalized = self.sync_state.current();
        debug!(
            "Last voted for: {:}, last finalized attestation: {:}",
            last_voted_for_block, last_finalized
        );
        // set last voted for
        metric_set_labels!(
            self.metrics,
            last_voted_for,
            [self.chain_name, self.chain_key],
            last_voted_for_block
        );

        // Determine drift baseline
        let baseline = if last_finalized == 0 && self.start_block > 0 {
            self.start_block
        } else {
            last_finalized
        };

        let vote_acceptance_window = self.vote_acceptance_window().await?;
        let diff = last_voted_for_block.saturating_sub(baseline);
        let drifted = diff > vote_acceptance_window;

        if last_voted_for_block < last_finalized {
            // no-op: service drains matured and catches up naturally
            info!("Votes behind finality; will catch up by draining heads");
        } else if drifted {
            warn!(
                   "⚠️ Attestation was finalized, but votes are ahead of finality. last_voted_for={:?}, last_finalized={}",
                   last_voted_for_block, last_finalized
               );
            info!(
                "⏸️ Pausing (backoff) due to drift; last_voted_for={:?}, epoch={}",
                last_voted_for_block, self.current_epoch
            );

            // Enter paused state
            self.state = State::PausedBackoff {
                since: std::time::Instant::now(),
                attempt: 0,
                total_paused: std::time::Duration::ZERO,
                reason: "drifted_ahead_of_finality",
            };

            // 🔴 Stop producing new heads while paused
            if let Some(h) = self.source_chain_task.take() {
                h.abort();
                let _ = h.await;
            }
            self.heads_rx = None;

            // 🔴 Drop any buffered work that was accumulated while we were drifting
            self.pending.clear();

            // schedule first backoff tick (use your jitter if you prefer)
            self.next_backoff_deadline = Some(Instant::now() + std::time::Duration::from_secs(10));
        }
        Ok(())
    }

    async fn drain_matured(&mut self) -> Result<()> {
        // Hard gate while paused: do not prepare/submit new attestations
        if self.state.is_paused_backoff() {
            info!(
                "Paused (backoff); holding {} pending headers",
                self.pending.values().map(std::vec::Vec::len).sum::<usize>()
            );
            return Ok(());
        }

        let last_eth_block_number = self.eth_client.get_last_block().await?;

        // update metric
        metric_set_labels!(
            self.metrics,
            source_chain_height,
            [self.chain_name, self.chain_key],
            last_eth_block_number
        );

        let mature = last_eth_block_number.saturating_sub(self.maturity_delay);
        let mut ready = Vec::new();
        while let Some((&k, _)) = self.pending.first_key_value() {
            if k > mature {
                break;
            }
            let (_, mut v) = self.pending.pop_first().unwrap();
            ready.append(&mut v); // move items into `ready`
        }

        if ready.is_empty() {
            info!(
                "👶 Attestations not matured yet, current eth head: {}",
                last_eth_block_number
            );
        }

        for block_header in ready {
            info!(
                "👨 Block({}) matured, going to prepare it for submission...",
                block_header
            );

            // prepare + submit
            match self.prepare_attestation(block_header).await {
                Ok(signed) => {
                    let digest = signed.digest();
                    // dedupe double-votes
                    let can_submit = self.voted_for.insert((block_header, digest));
                    if !can_submit {
                        info!("❗ Double vote detected for block: {} with digest: {:?}, skipping submission", block_header, digest);
                        continue;
                    }

                    let round = signed.round();
                    if let Err(e) = self
                        .cc3_client
                        .submit_attestation::<H256>(signed.clone())
                        .await
                    {
                        error!("Submit failed for round {:?}: {:?}", round, e);
                    } else {
                        info!(
                            "✉️ Submitted attestation round={:?}, digest={:?}, epoch={}",
                            round, digest, self.current_epoch
                        );
                        // publish for observers; can be enriched if needed
                        let _ = self.attestation_tx.send(signed);
                        // re-evaluate position
                        self.evaluate_voting_position().await.ok();
                    }
                }
                Err(e) => {
                    if e.is_not_selected_error() {
                        warn!("Failed to attest, attestor not selected.");
                    } else if e.is_fragment_error() {
                        panic!("Fragment error detected, exiting ...");
                    } else if e.is_attested_to_error() {
                        debug!("Attestation already submitted for round, skipping");
                    } else {
                        panic!("Failed to prepare attestation: {e:?}");
                    }
                }
            }
        }
        Ok(())
    }

    async fn note_cc_event(&mut self, event: CcEvent) -> Result<(), Error> {
        match event {
            CcEvent::AttestationIntervalChanged(_chain_key, interval) => {
                info!(
                    "📢 Attestation interval updated. New interval: {}",
                    interval
                );
                self.note_interval_change(interval).await?;
            }
            CcEvent::BlockAttested(attestation) => {
                let header = attestation.header_number();
                info!(
                    "📝 Block({}) attested for, digest: {:?}",
                    header,
                    attestation.digest()
                );
                self.note_last_attested_header(header).await?;
            }
            CcEvent::RandomnessChanged((epoch, randomness)) => {
                info!(
                    "🕒 Epoch rotated. Epoch: {}, Randomness: {}",
                    epoch,
                    hex::encode(randomness)
                );
                self.note_epoch_change(epoch).await?;
            }
            CcEvent::CheckpointReached(_ck, checkpoint) => {
                info!(
                    "✅ Checkpoint reached, block: {:}, digest: {:}",
                    checkpoint.block_number, checkpoint.digest
                );
                self.continuity_cache
                    .prune_all_before(checkpoint.block_number);
                self.voted_for
                    .retain(|(header_number, _)| *header_number > checkpoint.block_number);
            }
        }
        Ok(())
    }

    async fn note_interval_change(&mut self, new_interval: u64) -> Result<(), Error> {
        let needs_restart = self.cc3_client.get_attestation_interval() != new_interval;
        if needs_restart {
            self.cc3_client.change_attestation_interval(new_interval);
            info!(
                "🔀 Attestation interval changed to {}, restarting service",
                new_interval
            );
            self.start_running().await?;
        }
        Ok(())
    }

    async fn note_last_attested_header(&mut self, header: u64) -> Result<(), Error> {
        let last_voted_for = self.voted_for.last().copied().unwrap_or_default().0;

        debug!(
            "Last finalized attestation: {:}, last voted for: {:}",
            header, last_voted_for
        );
        metric_set_labels!(
            self.metrics,
            last_finalized_attestation,
            [self.chain_name, self.chain_key],
            header
        );

        if header == self.start_block {
            info!(
                "🟢 Genesis block attestation finalized, resuming from block: {:}",
                header
            );
            self.start_running().await?;
            return Ok(());
        }

        // Special handling for genesis block attestation
        // If we're paused at genesis and receive an attestation for the genesis block, resume
        if self.state.is_paused_backoff() && header == self.start_block {
            info!(
                "🟢 Genesis block attestation finalized, resuming from block: {:}",
                header
            );
            self.start_running().await?;
        } else if header
            >= last_voted_for.saturating_sub(
                ATTESTATIONS_RESTART_WINDOW * self.cc3_client.get_attestation_interval(),
            )
            && self.state.is_paused_backoff()
        {
            info!(
                "🟢 Chain caught up, resuming attestation at block: {:}",
                last_voted_for
            );
            self.start_running().await?;
        }

        // Update the sync state
        let last_eth_height = self.eth_client.get_last_block().await?;
        self.sync_state.update(header, last_eth_height);

        // If running, finality advanced — safe to drain matured
        if matches!(self.state, State::Running) {
            self.drain_matured().await?;
        }

        Ok(())
    }

    async fn note_epoch_change(&mut self, epoch: u64) -> Result<(), Error> {
        debug!("Noting current epoch: {}", epoch);
        self.current_epoch = epoch;
        metric_set_labels!(
            self.metrics,
            cc_current_epoch,
            [self.chain_name, self.chain_key],
            epoch
        );

        match self.state {
            State::Running => return Ok(()),
            State::PausedBackoff { .. } => {
                debug!("🔁 Epoch changed; nudging backoff");
                self.next_backoff_deadline = Some(Instant::now());
            }
            _ => {
                self.start_running().await?;
            }
        }
        Ok(())
    }

    /// Calculate the number of attestations between checkpoints
    async fn vote_acceptance_window(&self) -> Result<u64> {
        let attestation_interval = self.cc3_client.get_attestation_interval();
        let checkpoint_interval = u64::from(self.cc3_client.get_checkpoint_interval().await?);

        let vote_acceptance_window = self
            .cc3_client
            .get_vote_acceptance_window(self.chain_key)
            .await?;

        Ok(attestation_interval * checkpoint_interval * vote_acceptance_window)
    }

    /// Prepare an attestation for submission
    async fn prepare_attestation(
        &mut self,
        header_number: u64,
    ) -> Result<Attestation<H256, AttestorId>, Error> {
        // Eligiblity check
        let vrf_output = self.cc3_client.sign_vrf(header_number).await.map_err(|e| {
            debug!("Error signing vrf: {:?}", e);
            Error::NotSelected(header_number)
        })?;

        // Create continuity fragment
        let continuity_fragment = self.create_continuity_proof(header_number).await?;

        // Get the eth block for the header number
        let block = self
            .eth_client
            .get_block(header_number, self.encoding)
            .await?;
        // Get the previous digest from the continuity fragment
        let prev_digest = continuity_fragment.head_digest().copied();
        // Create attestation data
        let attestation = crate::util::create_attestation(self.chain_key, &block, prev_digest);

        // Serialize the fragment to be sent over the wire
        let serialized_fragment =
            continuity::AttestationFragmentSerializable::from(&continuity_fragment);

        // Sign the attestation
        let current_epoch = self.cc3_client.get_current_epoch().await?;
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
        if attestation_header_number == 0 || attestation_header_number == self.start_block {
            info!("🛠️ Creating default continuity proof for header number 0");
            return Ok(continuity::AttestationFragment::default());
        }

        // Get last attested source chain block number
        let result = get_last_finalized(&self.cc3_client, self.chain_key).await?;

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

    // ===== Backoff helpers (bridge to retry.rs expectations) =====
    pub fn chain_key(&self) -> ChainKey {
        self.chain_key
    }
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
