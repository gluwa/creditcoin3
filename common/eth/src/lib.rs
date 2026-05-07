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

/// One ordered fallback RPC provider attached to a [`Client`].
///
/// The [`Client`] always tries its primary URL first; on `Ok(None)` or
/// transport errors it walks `fallback_providers` in declared order and
/// uses the first non-empty answer. All fallback URLs must point to a
/// node that reports the same `chain_id` as the primary URL — this is
/// verified when the [`Client`] is constructed.
#[derive(Debug, Clone)]
pub(crate) struct FallbackProvider {
    pub(crate) url: Url,
    pub(crate) provider: AlloyProvider,
}

#[derive(Debug, Clone)]
pub struct Client {
    url: Url,
    private_key: Option<String>,
    rpc_provider: AlloyProvider,
    /// Ordered fallback RPC providers. Empty = no fallbacks (single-URL
    /// behavior). Each provider is tried in declared order whenever the
    /// primary returns `Ok(None)` or errors. See [`FallbackProvider`].
    fallback_providers: Vec<FallbackProvider>,
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
            fallback_providers: Vec::new(),
            chain_id,
            #[cfg(feature = "block_cache")]
            cache: None,
        })
    }

    /// Build a [`Client`] with ordered fallback RPC URLs.
    ///
    /// `url` is the primary URL: tried first for every operation, and the
    /// only URL used for tip-related calls (subscriptions, `eth_blockNumber`,
    /// `eth_chainId`).
    ///
    /// `fallback_urls` are tried in declaration order whenever the primary
    /// returns `Ok(None)` or a transport error for a block fetch / tx-hash
    /// lookup. Each fallback URL is connected to at startup and must report
    /// the same `chain_id` as `url`.
    ///
    /// When `fallback_urls` is empty this is equivalent to [`Client::new`].
    pub async fn new_with_fallbacks(
        url: &str,
        fallback_urls: &[String],
        private_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(url).await?;
        let fallback_providers = Self::init_fallback_providers(chain_id, fallback_urls).await?;

        anyhow::Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            rpc_provider,
            fallback_providers,
            chain_id,
            #[cfg(feature = "block_cache")]
            cache: None,
        })
    }

    pub async fn reconnect(&mut self) -> Result<(), Error> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(self.url.as_ref()).await?;

        // Reconnect each fallback against its own URL too, otherwise a
        // recovered primary would silently keep using a stale fallback
        // socket until that fallback hit a fault on its own.
        //
        // Fallback reconnects are best-effort: a healthy primary should not be
        // taken down because a backup endpoint is misbehaving. If a fallback
        // fails to reconnect (transport error or chain_id mismatch), keep the
        // existing provider in place, log a loud error, and let the next
        // primary-failure path retry it on its own.
        let mut new_fallbacks = Vec::with_capacity(self.fallback_providers.len());
        for (idx, fp) in self.fallback_providers.iter().enumerate() {
            match Self::init_rpc(fp.url.as_ref()).await {
                Ok((fp_url, fp_provider, fp_chain_id)) => {
                    if fp_chain_id != chain_id {
                        tracing::error!(
                            fallback_index = idx,
                            fallback_url = %redact_url_query(fp.url.as_str()),
                            fallback_chain_id = fp_chain_id,
                            primary_chain_id = chain_id,
                            "⛔ Fallback RPC chain_id mismatch on reconnect; keeping previous fallback provider"
                        );
                        new_fallbacks.push(fp.clone());
                    } else {
                        new_fallbacks.push(FallbackProvider {
                            url: fp_url,
                            provider: fp_provider,
                        });
                    }
                }
                Err(err) => {
                    tracing::error!(
                        fallback_index = idx,
                        fallback_url = %redact_url_query(fp.url.as_str()),
                        error = %err,
                        "⛔ Failed to reconnect fallback RPC; keeping previous fallback provider"
                    );
                    new_fallbacks.push(fp.clone());
                }
            }
        }

        self.url = url;
        self.rpc_provider = rpc_provider;
        self.fallback_providers = new_fallbacks;
        self.chain_id = chain_id;

        Ok(())
    }

    /// Connect to each fallback URL in declaration order and verify each
    /// reports the same `chain_id` as the primary.
    pub(crate) async fn init_fallback_providers(
        primary_chain_id: u64,
        fallback_urls: &[String],
    ) -> anyhow::Result<Vec<FallbackProvider>> {
        let mut providers: Vec<FallbackProvider> = Vec::with_capacity(fallback_urls.len());
        for (idx, raw_url) in fallback_urls.iter().enumerate() {
            let (url, provider, chain_id) =
                Self::init_rpc(raw_url.as_ref()).await.with_context(|| {
                    format!(
                        "Failed to connect to fallback RPC URL #{idx} ({})",
                        redact_url_query(raw_url),
                    )
                })?;

            if chain_id != primary_chain_id {
                anyhow::bail!(
                    "Fallback RPC URL #{idx} ({}) reports chain_id {chain_id}, \
                     which does not match the primary URL's chain_id {primary_chain_id}; \
                     all fallback URLs must point to the same chain",
                    redact_url_query(raw_url),
                );
            }

            providers.push(FallbackProvider { url, provider });
        }

        Ok(providers)
    }

    /// Build the ordered `[(label, provider)]` list used by the sequential
    /// fallback walk. The primary is first; each fallback gets a label like
    /// `fallback[0]@<redacted-url>` for log output.
    fn providers_with_labels(&self) -> Vec<(String, &AlloyProvider)> {
        let mut out: Vec<(String, &AlloyProvider)> =
            Vec::with_capacity(1 + self.fallback_providers.len());
        out.push(("primary".to_string(), &self.rpc_provider));
        for (i, fp) in self.fallback_providers.iter().enumerate() {
            out.push((
                format!("fallback[{i}]@{}", redact_url_query(fp.url.as_str())),
                &fp.provider,
            ));
        }
        out
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
        let providers = self.providers_with_labels();
        let mut got_definitive_none = false;
        let mut errors: Vec<(String, Error)> = Vec::new();

        for (label, provider) in providers {
            match provider
                .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(number)))
                .await
            {
                Ok(Some(receipts)) => {
                    if label != "primary" {
                        info!(
                            provider = %label,
                            block_number = number,
                            "receipts fetched via fallback provider"
                        );
                    }
                    return Ok(receipts);
                }
                Ok(None) => got_definitive_none = true,
                Err(e) => errors.push((label, Error::from(e))),
            }
        }

        // Mirror the legacy behavior: an `Ok(None)` from a single provider
        // mapped to `FailedToGetBlock(number)`. When at least one provider
        // says the receipts are not available we honor that and surface the
        // block-not-found error; transport errors from any other provider
        // are demoted to warnings.
        match merge_provider_lookup(got_definitive_none, errors) {
            LookupOutcome::NotFound { errors_to_warn } => {
                for (label, err) in errors_to_warn {
                    tracing::warn!(
                        provider = %label,
                        block_number = number,
                        error = %err,
                        "receipts fetch: provider errored but another said `not found`; treating as not found"
                    );
                }
                Err(Error::FailedToGetBlock(number))
            }
            LookupOutcome::AllErrored {
                first,
                additional_to_warn,
            } => {
                for (label, err) in additional_to_warn {
                    tracing::warn!(
                        provider = %label,
                        block_number = number,
                        error = %err,
                        "receipts fetch: additional provider error"
                    );
                }
                let (first_label, first_err) = first;
                error!(
                    provider = %first_label,
                    block_number = number,
                    error = %first_err,
                    "Failed to get receipts: all providers errored",
                );
                Err(Error::FailedToGetReceipts(number))
            }
        }
    }

    pub async fn get_eth_block(&self, number: u64) -> Result<Block, Error> {
        let providers = self.providers_with_labels();
        let mut got_definitive_none = false;
        let mut errors: Vec<(String, Error)> = Vec::new();

        for (label, provider) in providers {
            match provider
                .get_block(
                    BlockId::Number(BlockNumberOrTag::Number(number)),
                    true.into(),
                )
                .await
            {
                Ok(Some(block)) => {
                    if label != "primary" {
                        info!(
                            provider = %label,
                            block_number = number,
                            "block fetched via fallback provider"
                        );
                    }
                    return Ok(block);
                }
                Ok(None) => got_definitive_none = true,
                Err(e) => errors.push((label, Error::from(e))),
            }
        }

        match merge_provider_lookup(got_definitive_none, errors) {
            LookupOutcome::NotFound { errors_to_warn } => {
                for (label, err) in errors_to_warn {
                    tracing::warn!(
                        provider = %label,
                        block_number = number,
                        error = %err,
                        "block fetch: provider errored but another said `not found`; treating as not found"
                    );
                }
                Err(Error::FailedToGetBlock(number))
            }
            LookupOutcome::AllErrored {
                first,
                additional_to_warn,
            } => {
                for (label, err) in additional_to_warn {
                    tracing::warn!(
                        provider = %label,
                        block_number = number,
                        error = %err,
                        "block fetch: additional provider error"
                    );
                }
                let (first_label, first_err) = first;
                error!(
                    provider = %first_label,
                    block_number = number,
                    error = %first_err,
                    "Failed to get block: all providers errored",
                );
                Err(Error::FailedToGetBlock(number))
            }
        }
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
    ///
    /// # Routing
    ///
    /// A transaction hash carries no block-number information until it is
    /// resolved, so the lookup walks the configured providers in
    /// `[primary, fallback_0, fallback_1, ...]` order: the first provider
    /// to return `Ok(Some(_))` wins, and the rest are not queried. This
    /// minimizes calls to the (typically expensive / quota-limited) archive
    /// fallbacks: a recent tx that the primary can answer never reaches an
    /// archive endpoint at all.
    ///
    /// Result-merging policy:
    /// * The first provider to return `Ok(Some(_))` wins.
    /// * If every provider returns `Ok(None)`, the result is `Ok(None)`
    ///   (the tx truly does not exist on this chain).
    /// * If some providers return `Ok(None)` and others error, the errors
    ///   are logged as warnings and `Ok(None)` is returned — at least one
    ///   provider that did respond said the tx is not present.
    /// * If every provider errors, the first error is returned and the
    ///   rest are logged at warn level.
    pub async fn get_tx_position_by_hash(
        &self,
        tx_hash: H256,
    ) -> Result<Option<(u64, u64)>, Error> {
        let tx_hash_alloy = TxHash::from_str(&tx_hash.encode_hex())
            .map_err(|e| Error::ClientError(anyhow::anyhow!("Invalid tx hash: {e}")))?;

        let providers = self.providers_with_labels();
        let mut got_definitive_none = false;
        let mut errors: Vec<(String, Error)> = Vec::new();

        for (label, provider) in providers {
            match Self::resolve_tx_on_provider(provider, tx_hash_alloy, &tx_hash).await {
                Ok(Some(pos)) => {
                    // Only log when a fallback served the request; the
                    // primary case is the boring path and would otherwise
                    // dominate logs for projects without fallbacks.
                    //
                    // INFO level here (vs DEBUG for per-block logs)
                    // because tx-hash resolution is at most once per
                    // `/proof-by-tx` request, so log volume is bounded
                    // and the signal is high — operators want to confirm
                    // at a glance which provider answered.
                    if label != "primary" {
                        info!(
                            provider = %label,
                            tx_hash = %tx_hash,
                            block_number = pos.0,
                            tx_index = pos.1,
                            "tx-hash resolved by fallback provider"
                        );
                    }
                    return Ok(Some(pos));
                }
                Ok(None) => got_definitive_none = true,
                Err(e) => errors.push((label, e)),
            }
        }

        match merge_provider_lookup(got_definitive_none, errors) {
            LookupOutcome::NotFound { errors_to_warn } => {
                for (label, err) in errors_to_warn {
                    tracing::warn!(
                        provider = %label,
                        tx_hash = %tx_hash,
                        error = %err,
                        "tx-hash lookup: provider errored but another said `not found`; treating as not found"
                    );
                }
                Ok(None)
            }
            LookupOutcome::AllErrored {
                first,
                additional_to_warn,
            } => {
                for (label, err) in additional_to_warn {
                    tracing::warn!(
                        provider = %label,
                        tx_hash = %tx_hash,
                        error = %err,
                        "tx-hash lookup: additional provider error"
                    );
                }
                let (first_label, first_err) = first;
                tracing::error!(
                    provider = %first_label,
                    tx_hash = %tx_hash,
                    error = %first_err,
                    "tx-hash lookup: all providers errored"
                );
                Err(first_err)
            }
        }
    }

    /// One provider's slice of [`Self::get_tx_position_by_hash`]: looks the tx
    /// up by hash and converts it to `(block_number, tx_index)`. Pure
    /// extraction so the sequential walk stays readable.
    async fn resolve_tx_on_provider(
        provider: &AlloyProvider,
        tx_hash_alloy: TxHash,
        tx_hash_for_err: &H256,
    ) -> Result<Option<(u64, u64)>, Error> {
        let Some(tx) = provider
            .get_transaction_by_hash(tx_hash_alloy)
            .await
            .map_err(Error::from)?
        else {
            return Ok(None);
        };

        let block_number = tx.block_number.ok_or_else(|| {
            Error::ClientError(anyhow::anyhow!(
                "Transaction not in a block (pending): {tx_hash_for_err}"
            ))
        })?;
        let tx_index = tx.transaction_index.ok_or_else(|| {
            Error::ClientError(anyhow::anyhow!(
                "Missing transactionIndex for tx: {tx_hash_for_err}"
            ))
        })?;

        Ok(Some((block_number, tx_index)))
    }
}

/// Decision returned by [`merge_provider_lookup`] once the sequential
/// fallback walk finishes without any provider returning `Ok(Some(_))`.
///
/// The outer call site short-circuits on the first `Ok(Some(_))`, so this
/// only models the negative paths.
enum LookupOutcome {
    /// At least one provider answered "not found" (`Ok(None)`). Any errors
    /// from peers are demoted to warnings — the providers that did respond
    /// agreed the value is not there.
    NotFound {
        errors_to_warn: Vec<(String, Error)>,
    },
    /// Every provider errored. Caller gets `first` as the representative
    /// failure; remaining errors should be logged at warn level.
    AllErrored {
        first: (String, Error),
        additional_to_warn: Vec<(String, Error)>,
    },
}

/// Pure decision step shared by every sequential-fallback lookup
/// ([`Client::get_tx_position_by_hash`], [`Client::get_eth_block`],
/// [`Client::get_receipts`]).
///
/// `got_definitive_none` is `true` iff at least one provider returned
/// `Ok(None)`. `errors` carries every provider that failed to respond.
///
/// The caller is expected to have already returned early on the first
/// `Ok(Some(_))`, so this function only sees the negative arms.
///
/// # Panics
///
/// Panics if both `!got_definitive_none` and `errors.is_empty()`. That state
/// means "no provider returned anything", which is only possible if the
/// caller iterated an empty provider list — and the [`Client`] is constructed
/// such that there is always at least the primary.
fn merge_provider_lookup(
    got_definitive_none: bool,
    mut errors: Vec<(String, Error)>,
) -> LookupOutcome {
    if got_definitive_none {
        return LookupOutcome::NotFound {
            errors_to_warn: errors,
        };
    }

    assert!(
        !errors.is_empty(),
        "merge_provider_lookup invoked without any provider results"
    );

    let first = errors.remove(0);
    LookupOutcome::AllErrored {
        first,
        additional_to_warn: errors,
    }
}

/// Redact the query-string **and** any secret-looking path segments of a URL
/// for logs.
///
/// Many RPC providers carry their API key in the URL — either in the
/// `?key=...` query parameter (Google Cloud, Infura legacy) or as a trailing
/// path segment (Chainstack, Alchemy, QuickNode, …). This helper handles
/// both:
///
/// * Anything after the first `?` is replaced with `?…`.
/// * Any path segment that looks like an opaque secret token — i.e. at
///   least 16 characters of `[A-Za-z0-9_-]` containing at least one digit
///   *and* one letter — is replaced with `…` (the segment length is not
///   leaked).
///
/// Human-readable path tokens (`v1`, `projects`, `cc3-testnet-rpckey-2`,
/// `ethereum-sepolia`, etc.) stay intact because they either contain a
/// hyphen-separated word or are too short / not mixed-case to trip the
/// secret heuristic.
pub fn redact_url_query(url: &str) -> String {
    // Split off the query string first so we never accidentally redact
    // inside it (and to keep behavior identical to the previous helper for
    // `?key=...`-style URLs).
    let (base, query_marker) = match url.split_once('?') {
        Some((b, _)) => (b, "?…"),
        None => (url, ""),
    };

    // Redact the path. We split on '/' so we can scan each segment
    // independently; this keeps `scheme://host` untouched (the leading
    // `scheme:` and the empty pre-`//` segments don't match the secret
    // heuristic).
    let redacted_path = base
        .split('/')
        .map(|seg| {
            if looks_like_secret_segment(seg) {
                "…"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/");

    format!("{redacted_path}{query_marker}")
}

/// Heuristic: does this path segment look like an opaque secret token
/// (API key, project hash, …) that we should not log?
///
/// Conservative — designed to avoid false positives on routine path
/// components like `v1`, `rpc`, `ethereum-sepolia`, `cc3-testnet-rpckey-2`.
fn looks_like_secret_segment(seg: &str) -> bool {
    if seg.len() < 16 {
        return false;
    }

    // Hyphenated multi-word identifiers (e.g. `cc3-testnet-rpckey-2`,
    // `ethereum-sepolia`) are never secrets in practice — providers don't
    // shape API keys that way. This single rule covers all the
    // human-readable path tokens we've seen in proof-gen configs.
    if seg.contains('-') {
        return false;
    }

    let mut has_digit = false;
    let mut has_letter = false;
    for ch in seg.chars() {
        match ch {
            '0'..='9' => has_digit = true,
            'A'..='Z' | 'a'..='z' => has_letter = true,
            '_' => {}
            _ => return false, // any other char ⇒ not a secret-shaped token
        }
    }

    has_digit && has_letter
}

/// Build a simple Ethereum-compatible Merkle tree from a block
///
/// Uses `KeccakMerkleTree` which matches the POC implementation exactly.
pub fn simple_merkle_tree(block: &OrderedBlock) -> merkle::KeccakMerkleTree {
    let tx_bytes: Vec<Vec<u8>> = block.items().iter().map(|item| item.to_bytes()).collect();
    merkle::KeccakMerkleTree::new(&tx_bytes)
}

#[cfg(test)]
mod provider_lookup_tests {
    use super::{merge_provider_lookup, redact_url_query, Error, LookupOutcome};

    fn err(msg: &str) -> Error {
        Error::ClientError(anyhow::anyhow!(msg.to_string()))
    }

    #[test]
    fn at_least_one_none_yields_not_found_and_keeps_errors_for_warnings() {
        let errors = vec![
            ("primary".to_string(), err("connection reset")),
            ("fallback[0]@http://x".to_string(), err("rate limited")),
        ];
        let outcome = merge_provider_lookup(true, errors);
        match outcome {
            LookupOutcome::NotFound { errors_to_warn } => {
                assert_eq!(errors_to_warn.len(), 2);
                assert_eq!(errors_to_warn[0].0, "primary");
                assert_eq!(errors_to_warn[1].0, "fallback[0]@http://x");
            }
            LookupOutcome::AllErrored { .. } => panic!("expected NotFound"),
        }
    }

    #[test]
    fn definitive_none_with_no_errors_is_simple_not_found() {
        let outcome = merge_provider_lookup(true, Vec::new());
        match outcome {
            LookupOutcome::NotFound { errors_to_warn } => {
                assert!(errors_to_warn.is_empty());
            }
            LookupOutcome::AllErrored { .. } => panic!("expected NotFound"),
        }
    }

    #[test]
    fn no_none_and_some_errors_promotes_first_error() {
        let errors = vec![
            ("primary".to_string(), err("first failure")),
            ("fallback[0]@http://a".to_string(), err("second failure")),
            ("fallback[1]@http://b".to_string(), err("third failure")),
        ];
        let outcome = merge_provider_lookup(false, errors);
        match outcome {
            LookupOutcome::AllErrored {
                first,
                additional_to_warn,
            } => {
                assert_eq!(first.0, "primary");
                assert_eq!(first.1.to_string(), "Client error first failure");
                assert_eq!(additional_to_warn.len(), 2);
                assert_eq!(additional_to_warn[0].0, "fallback[0]@http://a");
                assert_eq!(additional_to_warn[1].0, "fallback[1]@http://b");
            }
            LookupOutcome::NotFound { .. } => panic!("expected AllErrored"),
        }
    }

    #[test]
    #[should_panic(expected = "merge_provider_lookup invoked without any provider results")]
    fn empty_input_panics() {
        // The caller iterates `[primary, fallbacks..]` so the input is never
        // empty in practice; assert the defensive check fires loudly if the
        // invariant is ever broken.
        let _ = merge_provider_lookup(false, Vec::new());
    }

    #[test]
    fn redact_url_query_strips_after_question_mark() {
        let redacted = redact_url_query("https://rpc.example.io/v2/foo?key=secret123");
        assert_eq!(redacted, "https://rpc.example.io/v2/foo?…");
    }

    #[test]
    fn redact_url_query_passes_through_short_path_when_no_query() {
        // No query, short path token — nothing to redact.
        let redacted = redact_url_query("wss://node.example.io/abcdef");
        assert_eq!(redacted, "wss://node.example.io/abcdef");
    }

    #[test]
    fn redact_url_query_redacts_chainstack_style_path_key() {
        // Chainstack embeds the API key as a 32-char hex path segment.
        let redacted = redact_url_query(
            "https://ethereum-sepolia.core.chainstack.com/efdb96b1ade73fac0eb3f90559b9acee",
        );
        assert_eq!(redacted, "https://ethereum-sepolia.core.chainstack.com/…");
    }

    #[test]
    fn redact_url_query_redacts_alchemy_style_trailing_token() {
        let redacted =
            redact_url_query("https://eth-mainnet.g.alchemy.com/v2/AbCdEf012345MoreSecret67890");
        // `v2` is short and stays; trailing token is redacted.
        assert_eq!(redacted, "https://eth-mainnet.g.alchemy.com/v2/…");
    }

    #[test]
    fn redact_url_query_keeps_human_readable_path_segments() {
        // Real-world googleapis URL — the path tokens are identifiers we
        // *want* to keep for log readability; the actual key sits in `?key=`.
        let redacted = redact_url_query(
            "https://blockchain.googleapis.com/v1/projects/cc3-testnet-rpckey-2/locations/us-central1/endpoints/ethereum-sepolia/rpc?key=AIzaSyVxWM",
        );
        assert_eq!(
            redacted,
            "https://blockchain.googleapis.com/v1/projects/cc3-testnet-rpckey-2/locations/us-central1/endpoints/ethereum-sepolia/rpc?…"
        );
    }

    #[test]
    fn redact_url_query_keeps_pure_letter_paths() {
        // No digits in the segment ⇒ not a key-shaped token; keep as-is.
        let redacted = redact_url_query("https://rpc.example.io/abcdefghijklmnopqrst");
        assert_eq!(redacted, "https://rpc.example.io/abcdefghijklmnopqrst");
    }

    #[test]
    fn redact_url_query_keeps_pure_digit_paths() {
        // No letters ⇒ likely a port/id/ts, not an API key.
        let redacted = redact_url_query("https://rpc.example.io/1234567890123456");
        assert_eq!(redacted, "https://rpc.example.io/1234567890123456");
    }

    #[test]
    fn redact_url_query_redacts_inner_secret_segment() {
        // Trailing `/rpc` is a fixed suffix; the secret is the segment in the
        // middle. Verify each segment is evaluated independently.
        let redacted = redact_url_query("https://eth.example.io/v1/aB3xQ9MnP2rT7vW4kY8jL6/rpc");
        assert_eq!(redacted, "https://eth.example.io/v1/…/rpc");
    }
}
