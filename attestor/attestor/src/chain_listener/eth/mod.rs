//! A [chain listener] responsible for synchronizing the latest block from an ethereum chain as
//! well as reacting to events in the [production worker].
//!
//! [chain listener]: crate::chain_listener
//! [production worker]: crate::worker::production

mod error;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
/// Configuration options for the ethereum [chain listener].
///
/// [chain listener]: crate::chain_listener
pub struct Config {
    /// Chain RPC url.
    eth_url: url::Url,
    /// Interval at which attestations are being produced and source chain blocks are synchronized.
    /// This value is fetched from on-chain storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    #[specify_later]
    pub attestation_interval: std::num::NonZero<common::types::Height>,
    /// Starting height at which attestation are produced and source chain block fetching begins.
    /// This value is fetched from on-chain storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    #[specify_later]
    pub start_height: common::types::Height,
}

// ----------------------------------------- [ Types ] ----------------------------------------- //

type AlloyProvider = alloy::providers::fillers::FillProvider<
    ExeFiller,
    alloy::providers::RootProvider<alloy::network::Ethereum>,
    alloy::network::Ethereum,
>;

type ExeFiller = alloy::providers::fillers::JoinFill<
    alloy::providers::Identity,
    alloy::providers::fillers::JoinFill<
        alloy::providers::fillers::GasFiller,
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::BlobGasFiller,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::NonceFiller,
                alloy::providers::fillers::ChainIdFiller,
            >,
        >,
    >,
>;

// ------------------------------------- [ Chain Listener ] ------------------------------------ //

/// Ethereum [chain listener], responsible for listening to new source chain blocks.
///
/// This listener is polled by the [production worker] to generate attestations as new source
/// chain blocks reach finality.
///
/// [chain listener]: crate::chain_listener
/// [production worker]: crate::worker::production
pub(crate) struct Ethereum {
    // SOURCE CHAIN
    eth: AlloyProvider,
    stream: alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>,
    catchup: super::Catchup,

    // CHAIN DATA
    chain_id: attestor_primitives::ChainId,
    attestation_interval: std::num::NonZero<common::types::Height>,
    start_height: common::types::Height,
}

impl Ethereum {
    /// Creates a new Ethereum [chain listener].
    ///
    /// [chain listener]: crate::chain_listener
    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use alloy::providers::Provider as _;
        use anyhow::Context as _;
        use futures::stream::StreamExt as _;

        tracing::info!("🛜 Staring Eth listener");
        tracing::info!(url = %config.eth_url, "🛜  with");

        // ------------------------------------* Configuration *-----------------------------------

        let eth = alloy::providers::ProviderBuilder::new()
            .network::<alloy::network::Ethereum>()
            .on_ws(alloy::providers::WsConnect::new(config.eth_url.clone()))
            .await
            .context("Failed to initialize Ethereum WS RPC connection")?;
        let mut stream = eth
            .subscribe_blocks()
            .await
            .context("Failed to initialize Ethereum WS block subscribtion")?
            .into_stream();
        let next_block = stream
            .next()
            .await
            .context("Unexpected end of stream")?
            .number
            .saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG);
        let catchup = super::Catchup {
            start: config.start_height,
            stop: next_block - (next_block % config.attestation_interval),
        };
        let chain_id = eth
            .get_chain_id()
            .await
            .context("Failed to retrive ethereum chain id")?;

        anyhow::Ok(Self {
            eth,
            stream,
            catchup,

            chain_id,
            attestation_interval: config.attestation_interval,
            start_height: config.start_height,
        })
    }

    /// Returns the next source chain block **height** to be attested to. Blocks are only returned
    /// once they have reached the [`ATTESTATION_FINALIZATION_LAG`].
    ///
    /// This will always return [`Some`] unless a manual user interrupt via `CTRL-C`has been
    /// observed.
    ///
    /// [`ATTESTATION_FINALIZATION_LAG`]: common::constants::ATTESTATION_FINALIZATION_LAG
    pub async fn next(&mut self) -> Option<Result<common::types::Height, Error>> {
        let attestation_interval = self.attestation_interval.get();

        loop {
            let start = self.catchup.start;

            if start <= self.catchup.stop {
                self.catchup.start += attestation_interval;
                break Some(Ok(start));
            } else {
                let block_n = match self.next_block().await {
                    Some(Ok(block_n)) => block_n,
                    fail => break fail,
                };

                tracing::debug!(block_n, "Updating catchup");

                if let Some(block_n) =
                    block_n.checked_sub(common::constants::ATTESTATION_FINALIZATION_LAG)
                {
                    self.catchup.stop = block_n;
                }
            }
        }
    }

    /// Retrieves the **full** source chain block at a given height.
    pub async fn block_get(
        &self,
        height: common::types::Height,
    ) -> Result<eth::OrderedBlock, Error> {
        use alloy::providers::Provider as _;

        let block_number = alloy::eips::BlockNumberOrTag::Number(height);
        let block_number = alloy::eips::BlockId::Number(block_number);

        let block_fut = self.eth.get_block(block_number, true.into());
        let receipts_fut = self.eth.get_block_receipts(block_number);

        let (block_res, receipts_res) = tokio::join!(block_fut, receipts_fut);

        let block = block_res
            .map_err(Error::RpcError)?
            .ok_or(Error::FetchBlock(height))?;
        let receipts = receipts_res
            .map_err(Error::RpcError)?
            .ok_or(Error::FetchBlockReceipts(height))?;

        ensure!(
            block.transactions.len() == receipts.len(),
            Error::FetchBlockReceiptsMismatch(height)
        );

        eth::OrderedBlock::try_create(
            self.chain_id,
            height,
            block.header.hash,
            block.transactions.into_transactions_vec(),
            receipts,
            ccnext_abi_encoding::common::EncodingVersion::V1,
        )
        .map_err(Error::OrderedBlockConversion)
    }

    pub fn block_latest(&self) -> common::types::Height {
        self.catchup.stop
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl crate::events::EventAttestationFinalizationAsync for Ethereum {
    type Error = std::convert::Infallible;

    /// A new attestation has reached finality on the execution chain.
    ///
    /// If we are catching up, we need to make sure we do not re-generate this attestation.
    async fn note_attestation_finalization_async(
        &mut self,
        attestation_latest_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        let (_digest, height) = attestation_latest_cc3;
        let target_start_new = util::next_multiple_of(self.attestation_interval, height);

        if self.catchup.start < target_start_new {
            self.catchup.start = target_start_new
        }

        Ok(())
    }
}
impl crate::events::EventAttestationFinalization for Ethereum {}

impl crate::events::EventAttestationIntervalChangeAsync for Ethereum {
    type Error = Error;

    /// A new attestation interval has been set on-chain.
    //
    /// We need to make sure the next source chain block we attest to is a multiple of the new
    /// attestation interval. If we are catching up on past attestations, we also need to make sure
    /// we skip any attestations before that point.
    async fn note_attestation_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        self.catchup.start = if let Some(attestation_latest_cc3) = attestation_latest_cc3 {
            util::next_multiple_of(interval_new, attestation_latest_cc3)
        } else {
            self.start_height
        };

        self.catchup.stop = loop {
            let Some(block_n) = self
                .next_block()
                .await
                .transpose()?
                .map(|n| n.saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG))
            else {
                // NOTE: INTERRUPT
                //
                // User-initiated shutdown
                return Ok(());
            };

            if block_n >= self.catchup.start {
                break block_n;
            }
        };

        self.attestation_interval = interval_new;

        tracing::debug!(
            attestation_interval = self.attestation_interval,
            catchup = ?self.catchup,
            "Updated attestation interval"
        );

        Ok(())
    }
}

// ----------------------------------------- [ HELPERS ] --------------------------------------- //

impl Ethereum {
    async fn next_block(&mut self) -> Option<Result<common::types::Height, Error>> {
        use alloy::providers::Provider as _;
        use futures::stream::StreamExt as _;

        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        loop {
            match self.stream.next().await {
                Some(block) => break Some(Ok(block.number)),
                None => match self.eth.subscribe_blocks().await {
                    Ok(sub) => self.stream = sub.into_stream(),
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to reconnect to eth, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            break Some(Err(Error::RpcError(err)));
                        }
                    }
                },
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => break None
            }

            delay = (delay * 2).min(DELAY_MAX);
        }
    }
}
