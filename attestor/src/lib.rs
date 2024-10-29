use anyhow::Result;
use attestor_primitives::Attestation;
use eth::Client;
use kameo::spawn;
use sp_core::H256;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

pub mod attestation;
pub mod cc3;
pub mod eth_sub;
pub mod merkle;

use cc_client::attestation::CcEvent;

#[derive(Debug, Clone)]
/// Attestor server is configured using `Config`
pub struct Server {
    config: Config,
}

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `eth_start_block`: Start block for the source chain
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
pub struct Config {
    pub eth_rpc_url: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    //pub bls_key: [u8; 32],
}

#[derive(Clone, Copy)]
pub struct IntervalUpdate {
    pub new_interval: u64,
    pub attested_height_at_change: u64,
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    pub async fn run(&self) -> Result<()> {
        let eth_client = Client::new(&self.config.eth_rpc_url, &String::new()).await?;
        let chain_id = eth_client.get_chain_id().await?;
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let mut cc3_client =
            cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key, chain_id).await?;
        cc3_client.init().await?;

        let attestation_interval = cc3_client.get_attestation_interval();

        let (mut attestation_tx, mut attestation_rx) = tokio::sync::mpsc::channel(1);

        // Start with an empty task
        let mut task: JoinHandle<_> = tokio::spawn(async {});
        // Check eligibility and start subscription if we are eligible
        // Otherwise, we will wait for the next randomness change event
        if check_elgibility(&cc3_client).await? {
            info!("Attestor eligible to start attesting!");
            task = subscribe_to_new_heads_task(
                eth_client.clone(),
                cc3_client.clone(),
                attestation_tx.clone(),
                attestation_interval,
            )
            .await?;
        } else {
            info!("Going to sleep because we are not eligible this epoch...");
        }

        // Start cc3 event subscription
        let chain_key = cc3_client.get_chain_key();
        let mut event_sub = cc3_client.cc_client.subscribe_events(chain_key)?;

        loop {
            // Biased tokio select, we will prioritze listening to randomness changed events
            // because this will re-evaluate the eligibility for attestors
            tokio::select! {
                biased;

                Some(event) = event_sub.next() => {
                    match event {
                        CcEvent::RandomnessChangedEvent((epoch, randomness)) => {
                            // Abort the previous task otherwise we will end up with multiple subscriptions
                            task.abort();

                            info!(
                                "Randomness changed: epoch {}, randomness: {}",
                                epoch,
                                hex::encode(randomness)
                            );

                            // Re-evaluate eligibility and start a new subscription
                            if check_elgibility(&cc3_client).await? {
                                info!("Attestor eligible to start attesting!");

                                // Must fetch attestation interval again, in case it changed
                                let attestation_interval = cc3_client.get_attestation_interval();

                                // reopen the channel
                                info!("Reopening attestation channel");
                                (attestation_tx, attestation_rx) = tokio::sync::mpsc::channel(1);

                                info!("Starting new subscription task");
                                task = subscribe_to_new_heads_task(
                                    eth_client.clone(),
                                    cc3_client.clone(),
                                    attestation_tx.clone(),
                                    attestation_interval,
                                ).await?;
                            } else {
                                info!("Going to sleep because we are not eligible this epoch...");
                                // drain channel
                                attestation_rx.close();
                                while (attestation_rx.recv().await).is_some() {
                                    info!("Draining attestation channel");
                                }
                            }
                        },
                        CcEvent::AttestationIntervalChangedEvent(_, new_interval, attested_height_at_change) => {
                            info!(
                                "Attestation interval updated. New interval: {:?}, Attested height at change: {:?}", new_interval, attested_height_at_change
                            );
                            let interval_update = IntervalUpdate {
                                new_interval,
                                attested_height_at_change,
                            };
                            cc3_client.change_attestation_interval(interval_update);
                        },
                        _ => ()
                    }
                },
                Some(attestation) = attestation_rx.recv() => {
                    info!("Received attestation to submit");
                    if let Some(attestation_to_submit) = attestation {
                        match cc3_client.submit_attestation(attestation_to_submit).await
                        {
                            Ok(()) => {}
                            Err(e) => {
                                error!("Failed to submit attestation: {:?}", e);
                            }
                        }
                    }
                }
            };
        }
    }
}

/// Checks if the attestor is eligible to attest
/// - Check if the attestor is still a member of the attestor set
/// - Check if the attestor can be included in the current epoch with the current randomness
async fn check_elgibility(cc3_client: &cc3::Client) -> Result<bool> {
    // First check if we are still an attestor member
    let is_attestor_member = cc3_client
        .cc_client
        .check_attestors_membership(cc3_client.get_chain_key())
        .await?;
    if !is_attestor_member {
        warn!("Attestor is not valid at current timeframe, cannot attest!");
        return Ok(false);
    };

    let result = cc3_client.sign_vrf().await;

    Ok(result.is_ok())
}

/// Subscribes to new Ethereum heads and starts the attestation process.
async fn subscribe_to_new_heads_task(
    eth_client: Client,
    cc3_client: cc3::Client,
    sender: Sender<Option<Attestation<H256>>>,
    attestation_interval: u64,
) -> Result<JoinHandle<()>> {
    let attestor = spawn(attestation::Attestor::default());
    let chain_key = cc3_client.get_chain_key();

    let last_attestation = cc3_client.get_last_attestation(chain_key).await?;

    let target_header = if let Some(last_attestation) = last_attestation {
        info!(
            "Last finalized attestation digest: {}",
            last_attestation.digest()
        );
        last_attestation.header_number() + attestation_interval
    } else {
        info!("No last attestation found, starting from 0");
        0
    };

    Ok(tokio::spawn(async move {
        info!(
            "Subscribing to new heads at target block: {}",
            target_header
        );
        if let Err(e) = eth_sub::subscribe_to_new_heads(
            eth_client,
            attestor,
            sender,
            target_header,
            attestation_interval,
            chain_key,
        )
        .await
        {
            debug!("Error in subscribing to new heads: {:?}", e);
        }
    }))
}
