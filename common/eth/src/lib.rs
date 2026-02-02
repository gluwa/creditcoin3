use alloy::{
    consensus::TxEnvelope,
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

use anyhow::Result;
use ccnext_abi_encoding::common::EncodingVersion;
use hex::FromHexError;
use sp_core::H256;
use std::str::FromStr;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tracing::{error, info, trace};
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
        ccnext_abi_encoding::abi::abi_encode(self.tx().clone(), self.rx().clone(), self.encoding)
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
    pub fn try_create(
        chain_id: u64,
        number: u64,
        hash: BlockHash,
        mut transactions: Vec<Transaction>,
        mut receipts: Vec<TransactionReceipt>,
        encoding: EncodingVersion,
    ) -> Result<Self, ConversionError> {
        transactions.sort_by_key(|tx| tx.transaction_index);
        receipts.sort_by_key(|rx| rx.transaction_index);

        let items = transactions
            .into_iter()
            .zip(receipts.into_iter())
            .map(|tx_rx| TxRx::try_create(tx_rx.0, tx_rx.1, encoding))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            chain_id,
            number,
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

#[derive(Debug, Clone)]
pub struct Client {
    url: Url,
    private_key: Option<String>,
    rpc_provider: AlloyProvider,
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
                return Err(anyhow::anyhow!(
                    "Unsupported URL scheme. Please use http(s):// or ws(s)://. Found: {url_scheme}"
                )
                .into());
            }
        };

        info!("Connecting to Ethereum node at {}", url);

        let chain_id = rpc_provider.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId(e.to_string())
        })?;

        Ok((url, rpc_provider, chain_id))
    }

    pub async fn new(url: &str, private_key: Option<&str>) -> Result<Self, Error> {
        let (url, rpc_provider, chain_id) = Self::init_rpc(url).await?;

        Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            rpc_provider,
            chain_id,
            #[cfg(feature = "block_cache")]
            cache: None,
        })
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
    ) -> Option<Result<OrderedBlock, Error>> {
        trace!(
            "Getting block {:?}",
            BlockId::Number(BlockNumberOrTag::Number(number))
        );

        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        let (block, receipts) = loop {
            let get_eth_block_fut = self.get_eth_block(number);
            let get_eth_receipts_fut = self.get_receipts(number);

            match futures::future::try_join(get_eth_block_fut, get_eth_receipts_fut).await {
                Ok((block, receipts)) => break (block, receipts),
                Err(err) => {
                    attempt += 1;

                    tracing::debug!(
                        attempt,
                        MAX_ATTEMPTS,
                        "Failed to retreive eth block, retrying..."
                    );

                    if attempt >= MAX_ATTEMPTS {
                        return Some(Err(err));
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {},
                _ = tokio::signal::ctrl_c() => return None
            }

            delay = (delay * 2).min(DELAY_MAX);
        };

        if block.transactions.len() != receipts.len() {
            return Some(Err(Error::TransactionsReceiptsMismatch(number)));
        }

        let transactions = block.transactions.into_transactions().collect::<Vec<_>>();

        let block = OrderedBlock::try_create(
            self.chain_id,
            number,
            block.header.hash,
            transactions,
            receipts,
            encoding,
        )
        .map_err(Error::TransactionConversion);

        Some(block)
    }

    #[cfg(not(feature = "block_cache"))]
    pub async fn get_block(
        &self,
        number: u64,
        encoding: EncodingVersion,
    ) -> Option<Result<OrderedBlock, Error>> {
        Self::try_fetch_block(self, number, encoding).await
    }

    pub async fn subscribe(
        &self,
    ) -> std::result::Result<alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>, Error>
    {
        Ok(self.rpc_provider.subscribe_blocks().await?.into_stream())
    }

    async fn get_receipts(&self, number: u64) -> Result<Vec<TransactionReceipt>, Error> {
        self.rpc_provider
            .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(number)))
            .await
            .map_err(|e| {
                error!("Failed to get receipts: {:?}", e);
                Error::FailedToGetReceipts(number)
            })?
            .ok_or(Error::FailedToGetBlock(number))
    }

    pub async fn get_eth_block(&self, number: u64) -> Result<Block, Error> {
        self.rpc_provider
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
    pub async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64), Error> {
        // Convert sp_core::H256 to alloy TxHash via hex string
        let tx_hash_alloy = TxHash::from_str(&tx_hash.encode_hex())
            .map_err(|e| Error::ClientError(anyhow::anyhow!("Invalid tx hash: {e}")))?;

        // Fetch the transaction by hash
        let tx_opt = self
            .rpc_provider
            .get_transaction_by_hash(tx_hash_alloy)
            .await
            .map_err(Error::from)?;

        let tx = tx_opt.ok_or_else(|| {
            Error::ClientError(anyhow::anyhow!("Transaction not found for hash {tx_hash}"))
        })?;

        // Extract block number and transaction index (both should be Some for mined tx)
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

        Ok((block_number, tx_index))
    }
}

/// Build a simple Ethereum-compatible Merkle tree from a block
///
/// Uses `KeccakMerkleTree` which matches the POC implementation exactly.
pub fn simple_merkle_tree(block: &OrderedBlock) -> merkle::KeccakMerkleTree {
    let tx_bytes: Vec<Vec<u8>> = block.items().iter().map(|item| item.to_bytes()).collect();
    merkle::KeccakMerkleTree::new(&tx_bytes)
}
