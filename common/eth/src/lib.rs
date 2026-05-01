use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        TxEnvelope,
    },
    hex::ToHexExt,
    network::{Ethereum, EthereumWallet},
    primitives::{BlockHash, TxHash},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        network::TransactionResponse,
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::{
        client::WsConnect,
        types::{
            eth::{Block, BlockId, BlockNumberOrTag},
            ConversionError, Transaction, TransactionReceipt,
        },
    },
    signers::{k256::ecdsa::SigningKey, local::PrivateKeySigner},
    transports::{http::reqwest::Url, TransportErrorKind},
};

use anyhow::{Context, Result};
use hex::FromHexError;
use sp_core::H256;
use std::str::FromStr;
use thiserror::Error;
use tracing::{error, info, trace};
use usc_abi_encoding::common::EncodingVersion;
use user::prelude::*;
use utils::block_item_traits::BlockItem;

pub use alloy::core::primitives::Address;

#[cfg(feature = "block_cache")]
pub mod block_cache;
#[cfg(feature = "block_cache")]
pub mod metrics;

pub mod continuity;
pub mod evm;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get block {0}")]
    FailedToGetBlock(u64),
    #[error("Failed to get receipts {0}")]
    FailedToGetReceipts(u64),
    #[error(
        "Computed transactions/receipts roots do not match block header for block {0} (possible reorg between RPC calls)"
    )]
    BlockHeaderRootsMismatch(u64),
    #[error(
        "Number of fetched transactions doesn't match number of fetched receipts for block {0}"
    )]
    TransactionsReceiptsMismatch(u64),
    #[error("Not full transactions fetched for block {0}")]
    NotFullTransactionsFetched(u64),
    #[error("Failed to get chain id, Error: {0}")]
    FailedToGetChainId(String),
    #[error("Ethereum RPC error {0}")]
    EthError(#[from] alloy::transports::RpcError<TransportErrorKind>),
    #[error("Client error {0}")]
    ClientError(#[from] anyhow::Error),
    #[error("Transaction conversion {0}")]
    TransactionConversion(ConversionError),
    #[error("End of subscription")]
    EndOfSubscription,
    #[error("Failed to get sync info")]
    FailedToGetSyncInfo,
    #[error("No Wallet configured")]
    NoWalletConfigured,
    #[error("Hex decoding error {0}")]
    HexDecodingError(#[from] FromHexError),
    #[error("Failed to get block by hash {0}")]
    FailedToGetBlockByHash(String),
    #[error("Failed to path rpc url {0}")]
    UrlParseError(#[from] url::ParseError),
    #[cfg(feature = "block_cache")]
    #[error("Redis error {0}")]
    RedisError(#[from] redis::RedisError),
    #[error("Unsupported URL scheme. Please use http(s):// or ws(s)://. Found: {0}")]
    UnsupportedUrl(String),
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "block_cache", derive(serde::Serialize, serde::Deserialize))]
pub struct TxRx {
    tx: Transaction,
    rx: TransactionReceipt,
    encoding: EncodingVersion,
}

impl TxRx {
    pub fn try_create(
        tx: Transaction,
        rx: TransactionReceipt,
        encoding: EncodingVersion,
    ) -> Result<Self, ConversionError> {
        Ok(Self { tx, rx, encoding })
    }

    pub fn tx(&self) -> &Transaction {
        &self.tx
    }

    pub fn rx(&self) -> &TransactionReceipt {
        &self.rx
    }

    pub fn tx_hash(&self) -> BlockHash {
        self.tx.tx_hash()
    }
}

impl BlockItem for TxRx {
    fn payload_bytes(&self) -> Vec<u8> {
        usc_abi_encoding::abi::abi_encode(self.tx().clone(), self.rx().clone(), self.encoding)
            .expect("Transaction and receipt should be encodable.")
            .abi()
            .to_vec()
    }

    fn tx_type(&self) -> Option<u8> {
        match self.tx.inner.clone() {
            TxEnvelope::Legacy(_) => None,
            TxEnvelope::Eip2930(_) => Some(1),
            TxEnvelope::Eip1559(_) => Some(2),
            TxEnvelope::Eip4844(_) => Some(3),
            TxEnvelope::Eip7702(_) => Some(4),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "block_cache", derive(serde::Serialize, serde::Deserialize))]
pub struct OrderedBlock {
    chain_id: u64,
    number: u64,
    hash: BlockHash,
    items: Vec<TxRx>,
}

impl OrderedBlock {
    /// Builds an [`OrderedBlock`] from RPC-fetched [`Block`] and receipts. Verifies that
    /// recomputed transaction and receipt Merkle roots match the header (so a reorg between
    /// `eth_getBlockByNumber` and `eth_getBlockReceipts` cannot produce a mismatched attestation).
    /// Sorts transactions and receipts by `transaction_index` once.
    pub fn try_from_fetched_block(
        chain_id: u64,
        block: Block,
        mut receipts: Vec<TransactionReceipt>,
        expected_number: u64,
        encoding: EncodingVersion,
    ) -> Result<Self, Error> {
        if block.header.number != expected_number {
            return Err(Error::FailedToGetBlock(expected_number));
        }

        let hash = block.header.hash;

        // Empty blocks: many execution clients (incl. Substrate/Frontier dev chains) expose header
        // tx/receipt roots that do not match standard trie recomputation for an empty body, while the
        // fetched body and receipts are still consistent (both empty). There is no cross-fetch payload
        // to mis-associate, so we skip the header root check in this case.
        if block.transactions.is_empty() && receipts.is_empty() {
            trace!(
                block_number = expected_number,
                "Skipping header root check for empty block"
            );
            return Ok(Self {
                chain_id,
                number: expected_number,
                hash,
                items: vec![],
            });
        }

        if block.transactions.len() != receipts.len() {
            return Err(Error::TransactionsReceiptsMismatch(expected_number));
        }

        let mut txs: Vec<Transaction> = block.transactions.into_transactions().collect();

        if txs.iter().any(|t| t.transaction_index.is_none()) {
            return Err(Error::NotFullTransactionsFetched(expected_number));
        }
        if receipts.iter().any(|r| r.transaction_index.is_none()) {
            return Err(Error::FailedToGetReceipts(expected_number));
        }

        txs.sort_by_key(|tx| tx.transaction_index);
        receipts.sort_by_key(|rx| rx.transaction_index);

        let tx_inners: Vec<_> = txs.iter().map(|t| t.inner.clone()).collect();
        let computed_tx_root = calculate_transaction_root(&tx_inners);

        let inner_receipts: Vec<_> = receipts
            .iter()
            .map(|r| r.clone().into_primitives_receipt().inner)
            .collect();
        let computed_receipt_root = calculate_receipt_root(&inner_receipts);

        if computed_tx_root != block.header.transactions_root
            || computed_receipt_root != block.header.receipts_root
        {
            return Err(Error::BlockHeaderRootsMismatch(expected_number));
        }

        let items = txs
            .into_iter()
            .zip(receipts.into_iter())
            .map(|tx_rx| TxRx::try_create(tx_rx.0, tx_rx.1, encoding))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::TransactionConversion)?;

        Ok(Self {
            chain_id,
            number: expected_number,
            hash,
            items,
        })
    }
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
    pub fn number(&self) -> u64 {
        self.number
    }
    pub fn hash(&self) -> BlockHash {
        self.hash
    }
    pub fn items(&self) -> &[TxRx] {
        &self.items[..]
    }
}

pub struct OrderedRawBlock {
    pub chain_id: Option<u64>,
    pub number: u64,
    pub hash: BlockHash,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
}

impl OrderedRawBlock {
    pub fn new(
        chain_id: Option<u64>,
        number: u64,
        hash: BlockHash,
        mut transactions: Vec<Transaction>,
        mut receipts: Vec<TransactionReceipt>,
    ) -> Self {
        transactions.sort_by_key(|tx| tx.transaction_index);
        receipts.sort_by_key(|rx| rx.transaction_index);

        Self {
            chain_id,
            number,
            hash,
            transactions,
            receipts,
        }
    }
}

type AlloyProvider = FillProvider<ExeFiller, RootProvider<Ethereum>, Ethereum>;
pub type AlloyB256 = BlockHash;

pub(crate) type ExeFiller = JoinFill<
    Identity,
    JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
>;

type AlloyWalletProvider = FillProvider<WalletExeFiller, RootProvider<Ethereum>, Ethereum>;

pub(crate) type WalletExeFiller = JoinFill<
    JoinFill<
        alloy::providers::Identity,
        JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
    >,
    WalletFiller<EthereumWallet>,
>;

pub enum ConnectionTransport {
    Http(alloy::transports::http::reqwest::Url),
    Ws(WsConnect),
}

/// One per-block-range RPC URL override.
///
/// Block-keyed RPC requests (block fetch, receipts) whose `block_number`
/// falls within the inclusive range `[from_block, to_block]` are routed to
/// `url` instead of the [`Client`]'s default URL. Either bound may be `None`
/// to mean "open-ended on that side".
///
/// All override URLs must point to a node that reports the same `chain_id`
/// as the default URL — this is verified when the [`Client`] is constructed.
///
/// Constraints (enforced at construction time):
/// * Each override must specify at least one of `from_block` or `to_block`
///   (otherwise it would silently shadow the default URL).
/// * If both bounds are set, `from_block <= to_block`.
/// * Overrides must not overlap each other.
#[derive(Debug, Clone)]
pub struct RpcRangeOverride {
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RangeProvider {
    pub(crate) from: u64,
    pub(crate) to: u64,
    pub(crate) url: Url,
    pub(crate) provider: AlloyProvider,
}

#[derive(Debug, Clone)]
pub struct Client {
    url: Url,
    private_key: Option<String>,
    rpc_provider: AlloyProvider,
    /// Block-range RPC overrides, sorted by `from`. Empty = no overrides
    /// (legacy single-URL behavior). See [`RpcRangeOverride`].
    range_providers: Vec<RangeProvider>,
    // what chain id is implied here? Maybe need to define internal chain ids for different attestation chains
    // and not rely on ethereum chain ids?
    chain_id: u64,
    #[cfg(feature = "block_cache")]
    cache: Option<block_cache::Cache>,
}

impl Client {
    async fn init_rpc(url: &str) -> Result<(Url, AlloyProvider, u64), Error> {
        let url = Url::parse(url)?;
        let url_scheme = url.scheme();

        let rpc_provider = match url_scheme {
            "http" | "https" => ProviderBuilder::new()
                .network::<Ethereum>()
                .on_http(url.clone()),

            "ws" | "wss" => {
                let ws = WsConnect::new(url.clone());
                ProviderBuilder::new()
                    .network::<Ethereum>()
                    .on_ws(ws)
                    .await?
            }

            _ => {
                return Err(Error::UnsupportedUrl(url.to_string()));
            }
        };

        info!("🚀 🌐 Connecting to Ethereum node at {}", url);

        let chain_id = rpc_provider
            .get_chain_id()
            .await
            .context("Failed to get chain_id")?;

        Ok((url, rpc_provider, chain_id))
    }

    pub async fn new(url: &str, private_key: Option<&str>) -> anyhow::Result<Self> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(url).await?;

        anyhow::Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            rpc_provider,
            range_providers: Vec::new(),
            chain_id,
            #[cfg(feature = "block_cache")]
            cache: None,
        })
    }

    /// Build a [`Client`] with per-block-range RPC URL overrides.
    ///
    /// `url` is the default URL used for tip-related calls (subscriptions,
    /// `eth_blockNumber`, `eth_getTransactionByHash`) and for any block whose
    /// number is not covered by an entry in `overrides`.
    ///
    /// All override URLs are connected to at startup; each must report the
    /// same `chain_id` as `url` and the override ranges must satisfy the
    /// validity rules documented on [`RpcRangeOverride`].
    ///
    /// When `overrides` is empty this is equivalent to [`Client::new`].
    pub async fn new_with_overrides(
        url: &str,
        overrides: &[RpcRangeOverride],
        private_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(url).await?;
        let range_providers = Self::init_range_providers(chain_id, overrides).await?;

        anyhow::Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            rpc_provider,
            range_providers,
            chain_id,
            #[cfg(feature = "block_cache")]
            cache: None,
        })
    }

    pub async fn reconnect(&mut self) -> Result<(), Error> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(self.url.as_ref()).await?;

        // Reconnect each range provider against its own URL too, otherwise a
        // recovered default would silently start serving range-bucket blocks
        // until each override picked up the network fault on its own.
        let mut new_ranges = Vec::with_capacity(self.range_providers.len());
        for rp in &self.range_providers {
            let (rp_url, rp_provider, rp_chain_id) = Self::init_rpc(rp.url.as_ref()).await?;
            if rp_chain_id != chain_id {
                return Err(Error::ClientError(anyhow::anyhow!(
                    "RPC override URL chain_id ({rp_chain_id}) does not match default URL chain_id ({chain_id}) on reconnect"
                )));
            }
            new_ranges.push(RangeProvider {
                from: rp.from,
                to: rp.to,
                url: rp_url,
                provider: rp_provider,
            });
        }

        self.url = url;
        self.rpc_provider = rpc_provider;
        self.range_providers = new_ranges;
        self.chain_id = chain_id;

        Ok(())
    }

    /// Validate the override list (no network I/O), connect to each override
    /// URL in declaration order, and verify the chain id matches the default.
    pub(crate) async fn init_range_providers(
        default_chain_id: u64,
        overrides: &[RpcRangeOverride],
    ) -> anyhow::Result<Vec<RangeProvider>> {
        // Fail fast on invalid bounds before opening any sockets.
        let normalized = validate_range_overrides(overrides)?;

        let mut providers: Vec<RangeProvider> = Vec::with_capacity(normalized.len());
        for (from, to, raw_url) in normalized {
            let (url, provider, chain_id) =
                Self::init_rpc(raw_url.as_ref()).await.with_context(|| {
                    format!("Failed to connect to RPC override URL for block range [{from}, {to}]")
                })?;

            if chain_id != default_chain_id {
                anyhow::bail!(
                    "RPC override for block range [{from}, {to}] reports chain_id {chain_id}, \
                     which does not match the default URL's chain_id {default_chain_id}; \
                     all override URLs must point to the same chain"
                );
            }

            providers.push(RangeProvider {
                from,
                to,
                url,
                provider,
            });
        }

        Ok(providers)
    }

    /// Pick the RPC provider for a block-keyed call.
    ///
    /// Returns the matching range override's provider when one covers
    /// `block_number`, otherwise the default `rpc_provider`.
    fn provider_for_block(&self, block_number: u64) -> &AlloyProvider {
        let bounds: Vec<(u64, u64)> = self
            .range_providers
            .iter()
            .map(|rp| (rp.from, rp.to))
            .collect();
        match find_range_index(block_number, &bounds) {
            Some(i) => &self.range_providers[i].provider,
            None => &self.rpc_provider,
        }
    }

    /// Returns the configured RPC overrides as `(from, to, url)` tuples
    /// (sorted by `from`). Useful for startup logging.
    pub fn range_overrides(&self) -> Vec<(u64, u64, &Url)> {
        self.range_providers
            .iter()
            .map(|rp| (rp.from, rp.to, &rp.url))
            .collect()
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn get_url(&self) -> Result<ConnectionTransport> {
        let scheme = self.url.scheme();

        match scheme {
            "wss" | "ws" => {
                let ws = WsConnect::new(self.url.clone());
                Ok(ConnectionTransport::Ws(ws))
            }
            "https" | "http" => Ok(ConnectionTransport::Http(self.url.clone())),
            _ => Err(anyhow::anyhow!("Unsupported scheme: {scheme}")),
        }
    }

    /// Creates a provider that can be used to send transactions and query the Ethereum network.
    /// This provider contains the wallet configured with the private key.
    pub async fn get_wallet_ws_provider(&self) -> Result<AlloyWalletProvider, Error> {
        let builder = ProviderBuilder::new().wallet(EthereumWallet::from(self.get_signer()?));

        let provider = match self.get_url()? {
            ConnectionTransport::Http(url) => builder.on_http(url),
            ConnectionTransport::Ws(ws_client) => builder.on_ws(ws_client).await?,
        };

        Ok(provider)
    }

    pub fn get_signer(&self) -> Result<PrivateKeySigner, Error> {
        if self.private_key.is_none() {
            return Err(Error::NoWalletConfigured);
        }

        let decoded = hex::decode(self.private_key.clone().unwrap().replace("0x", ""))?;
        let signing_key = SigningKey::from_slice(&decoded).map_err(|e| {
            error!("Failed to create signing key: {:?}", e);
            Error::ClientError(anyhow::anyhow!("Failed to create signing key"))
        })?;

        Ok(PrivateKeySigner::from_signing_key(signing_key))
    }

    async fn try_fetch_block(
        &self,
        number: u64,
        encoding: EncodingVersion,
    ) -> Result<OrderedBlock, Interrupt<Error>> {
        trace!(
            "Getting block {:?}",
            BlockId::Number(BlockNumberOrTag::Number(number))
        );

        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        let ordered_block = loop {
            let get_eth_block_fut = self.get_eth_block(number);
            let get_eth_receipts_fut = self.get_receipts(number);

            match futures::future::try_join(get_eth_block_fut, get_eth_receipts_fut).await {
                Ok((block, receipts)) => {
                    match OrderedBlock::try_from_fetched_block(
                        self.chain_id,
                        block,
                        receipts,
                        number,
                        encoding,
                    ) {
                        Ok(ob) => break ob,
                        Err(err) => {
                            attempt += 1;
                            tracing::debug!(
                                attempt,
                                MAX_ATTEMPTS,
                                error = %err,
                                "Block body inconsistent with header roots (likely reorg between RPC calls), retrying..."
                            );
                            if attempt >= MAX_ATTEMPTS {
                                tracing::error!(error = %err, "⛔ Failed to verify consistent block data");
                                return Err(Interrupt::Cont(err));
                            }
                        }
                    }
                }
                Err(err) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retrieve eth block, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        tracing::error!(error = %err, "⛔ Failed to retrieve eth block");
                        return Err(Interrupt::Cont(err));
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {},
                _ = tokio::signal::ctrl_c() => return Err(Interrupt::Stop)
            }

            delay = (delay * 2).min(DELAY_MAX);
        };

        Ok(ordered_block)
    }

    #[cfg(not(feature = "block_cache"))]
    pub async fn get_block(
        &self,
        number: u64,
        encoding: EncodingVersion,
    ) -> Result<OrderedBlock, Interrupt<Error>> {
        Self::try_fetch_block(self, number, encoding).await
    }

    pub async fn subscribe(
        &self,
    ) -> std::result::Result<alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>, Error>
    {
        Ok(self.rpc_provider.subscribe_blocks().await?.into_stream())
    }

    async fn get_receipts(&self, number: u64) -> Result<Vec<TransactionReceipt>, Error> {
        self.provider_for_block(number)
            .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(number)))
            .await
            .map_err(|e| {
                error!("Failed to get receipts: {:?}", e);
                Error::FailedToGetReceipts(number)
            })?
            .ok_or(Error::FailedToGetBlock(number))
    }

    pub async fn get_eth_block(&self, number: u64) -> Result<Block, Error> {
        self.provider_for_block(number)
            .get_block(
                BlockId::Number(BlockNumberOrTag::Number(number)),
                true.into(),
            )
            .await
            .map_err(|e| {
                error!("Failed to get block: {:?}", e);
                Error::FailedToGetBlock(number)
            })?
            .ok_or(Error::FailedToGetBlock(number))
    }

    pub async fn get_last_block(&self) -> Result<u64, Error> {
        Ok(self.rpc_provider.get_block_number().await?)
    }

    pub async fn get_chain_id(&self) -> Result<u64, Error> {
        self.rpc_provider.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId(e.to_string())
        })
    }

    pub async fn get_block_number_by_hash(&self, hash: BlockHash) -> Result<u64, Error> {
        let block_opt = self
            .rpc_provider
            .get_block_by_hash(hash, true.into())
            .await
            .map_err(|e| {
                error!("Failed to get block by hash: {:?}", e);
                Error::FailedToGetBlockByHash(hash.to_string())
            })?;

        let block = block_opt.ok_or(Error::FailedToGetBlockByHash(hash.to_string()))?;

        Ok(block.header.number)
    }

    /// Resolve a transaction hash to its block number and index within the block.
    ///
    /// Returns `Ok(None)` if the transaction is not found on chain (not mined or doesn't exist).
    /// Returns `Err` only for actual RPC/transport failures.
    pub async fn get_tx_position_by_hash(
        &self,
        tx_hash: H256,
    ) -> Result<Option<(u64, u64)>, Error> {
        let tx_hash_alloy = TxHash::from_str(&tx_hash.encode_hex())
            .map_err(|e| Error::ClientError(anyhow::anyhow!("Invalid tx hash: {e}")))?;

        let tx_opt = self
            .rpc_provider
            .get_transaction_by_hash(tx_hash_alloy)
            .await
            .map_err(Error::from)?;

        let Some(tx) = tx_opt else {
            return Ok(None);
        };

        let block_number = tx.block_number.ok_or_else(|| {
            Error::ClientError(anyhow::anyhow!(
                "Transaction not in a block (pending): {tx_hash}"
            ))
        })?;
        let tx_index = tx.transaction_index.ok_or_else(|| {
            Error::ClientError(anyhow::anyhow!(
                "Missing transactionIndex for tx: {tx_hash}"
            ))
        })?;

        Ok(Some((block_number, tx_index)))
    }
}

/// Validate a slice of [`RpcRangeOverride`]s, returning normalized
/// `(from, to, url)` tuples sorted by `from`.
///
/// This performs *no* network I/O so it can be exercised cheaply in unit
/// tests and called as a pre-flight check in [`Client::init_range_providers`].
///
/// # Errors
///
/// * Either bound is `None` on both sides (would shadow the default URL).
/// * `from_block > to_block`.
/// * Any two overrides cover an overlapping block range.
fn validate_range_overrides(
    overrides: &[RpcRangeOverride],
) -> anyhow::Result<Vec<(u64, u64, String)>> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }

    let mut bounds: Vec<(u64, u64, String)> = Vec::with_capacity(overrides.len());
    for ov in overrides {
        if ov.from_block.is_none() && ov.to_block.is_none() {
            anyhow::bail!(
                "RPC range override must set at least one of `from_block` or `to_block` \
                 (an unbounded override would silently shadow the default URL)"
            );
        }
        let from = ov.from_block.unwrap_or(0);
        let to = ov.to_block.unwrap_or(u64::MAX);
        if from > to {
            anyhow::bail!(
                "RPC range override has from_block ({from}) > to_block ({to}); \
                 swap the bounds in your config"
            );
        }
        bounds.push((from, to, ov.url.clone()));
    }

    bounds.sort_by_key(|(from, _, _)| *from);
    for w in bounds.windows(2) {
        let (a_from, a_to, _) = &w[0];
        let (b_from, b_to, _) = &w[1];
        if a_to >= b_from {
            anyhow::bail!(
                "Overlapping RPC overrides: range [{a_from}, {a_to}] overlaps [{b_from}, {b_to}]"
            );
        }
    }

    Ok(bounds)
}

/// Find the index of the inclusive `(from, to)` range that covers
/// `block_number`.
///
/// Assumes `ranges` is sorted by `from` (as established by
/// [`validate_range_overrides`]) and that ranges are non-overlapping, so the
/// search short-circuits at the first range whose `from > block_number`.
fn find_range_index(block_number: u64, ranges: &[(u64, u64)]) -> Option<usize> {
    for (i, (from, to)) in ranges.iter().enumerate() {
        if block_number < *from {
            return None;
        }
        if block_number <= *to {
            return Some(i);
        }
    }
    None
}

/// Build a simple Ethereum-compatible Merkle tree from a block
///
/// Uses `KeccakMerkleTree` which matches the POC implementation exactly.
pub fn simple_merkle_tree(block: &OrderedBlock) -> merkle::KeccakMerkleTree {
    let tx_bytes: Vec<Vec<u8>> = block.items().iter().map(|item| item.to_bytes()).collect();
    merkle::KeccakMerkleTree::new(&tx_bytes)
}

#[cfg(test)]
mod range_override_tests {
    use super::{find_range_index, validate_range_overrides, RpcRangeOverride};

    fn ov(from: Option<u64>, to: Option<u64>, url: &str) -> RpcRangeOverride {
        RpcRangeOverride {
            from_block: from,
            to_block: to,
            url: url.to_string(),
        }
    }

    #[test]
    fn empty_overrides_validates_to_empty() {
        let normalized = validate_range_overrides(&[]).expect("empty list is valid");
        assert!(normalized.is_empty());
    }

    #[test]
    fn unbounded_override_is_rejected() {
        let err = validate_range_overrides(&[ov(None, None, "http://x")]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("at least one of `from_block` or `to_block`"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn inverted_bounds_are_rejected() {
        let err = validate_range_overrides(&[ov(Some(100), Some(50), "http://x")]).unwrap_err();
        assert!(
            err.to_string().contains("from_block"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn overlapping_overrides_are_rejected() {
        let err = validate_range_overrides(&[
            ov(Some(0), Some(1_000), "http://low"),
            ov(Some(500), Some(2_000), "http://mid"),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("Overlapping"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn touching_ranges_are_overlapping() {
        // Ranges are inclusive on both ends, so [0, 100] and [100, 200] both
        // cover block 100 — that ambiguity is rejected.
        let err = validate_range_overrides(&[
            ov(Some(0), Some(100), "http://low"),
            ov(Some(100), Some(200), "http://hi"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("Overlapping"));
    }

    #[test]
    fn adjacent_ranges_are_allowed() {
        let normalized = validate_range_overrides(&[
            ov(Some(0), Some(99), "http://low"),
            ov(Some(100), Some(200), "http://hi"),
        ])
        .expect("adjacent ranges must be allowed");
        assert_eq!(
            normalized,
            vec![
                (0, 99, "http://low".to_string()),
                (100, 200, "http://hi".to_string()),
            ]
        );
    }

    #[test]
    fn validation_normalizes_open_bounds_and_sorts() {
        let normalized = validate_range_overrides(&[
            ov(Some(4_000_001), None, "http://recent"),
            ov(None, Some(4_000_000), "http://archive"),
        ])
        .expect("non-overlapping open bounds are valid");

        assert_eq!(
            normalized,
            vec![
                (0, 4_000_000, "http://archive".to_string()),
                (4_000_001, u64::MAX, "http://recent".to_string()),
            ]
        );
    }

    #[test]
    fn find_range_index_picks_matching_range() {
        // Mirrors the user-facing two-bucket example: archive vs recent.
        let ranges = &[(0, 4_000_000), (4_000_001, u64::MAX)];

        assert_eq!(find_range_index(0, ranges), Some(0));
        assert_eq!(find_range_index(4_000_000, ranges), Some(0));
        assert_eq!(find_range_index(4_000_001, ranges), Some(1));
        assert_eq!(find_range_index(u64::MAX, ranges), Some(1));
    }

    #[test]
    fn find_range_index_returns_none_for_gap() {
        // Block 75 falls between [0, 50] and [100, 200] — no override matches,
        // so the caller falls back to the default URL.
        let ranges = &[(0, 50), (100, 200)];

        assert_eq!(find_range_index(75, ranges), None);
        assert_eq!(find_range_index(50, ranges), Some(0));
        assert_eq!(find_range_index(100, ranges), Some(1));
        assert_eq!(find_range_index(201, ranges), None);
    }

    #[test]
    fn find_range_index_with_no_ranges_returns_none() {
        assert_eq!(find_range_index(42, &[]), None);
    }
}
