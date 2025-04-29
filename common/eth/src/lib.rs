use alloy::{
    consensus::TxEnvelope,
    network::Ethereum,
    primitives::BlockHash,
    providers::{
        fillers::{BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller},
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
use hex::FromHexError;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tracing::{error, info};
use utils::{
    block_item_traits::{BlockItem, BlockItemIdentifier},
    StarknetPedersenMerkleTree,
};

pub use alloy::core::primitives::Address;

pub mod evm;
pub mod subscription;

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
    #[error("Failed to get chain id")]
    FailedToGetChainId,
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
    #[error("Failed to send block on channel")]
    SendError(#[from] SendError<OrderedBlock>),
    #[error("No Wallet configured")]
    NoWalletConfigured,
    #[error("Hex decoding error {0}")]
    HexDecodingError(#[from] FromHexError),
}

#[derive(Debug)]
pub struct TxRx {
    id: BlockItemIdentifier,
    tx: Transaction,
    rx: TransactionReceipt,
}

impl TxRx {
    pub fn try_create(
        id: BlockItemIdentifier,
        tx: Transaction,
        rx: TransactionReceipt,
    ) -> Result<Self, ConversionError> {
        Ok(Self { id, tx, rx })
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
        ccnext_abi_encoding::abi::abi_encode(self.tx().clone(), self.rx().clone())
            .expect("Transaction and receipt should be encodable.")
            .abi
    }

    fn id(&self) -> &BlockItemIdentifier {
        &self.id
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

#[derive(Debug)]
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
    ) -> Result<Self, ConversionError> {
        transactions.sort_by_key(|tx| tx.transaction_index);
        receipts.sort_by_key(|rx| rx.transaction_index);

        let items = transactions
            .into_iter()
            .zip(receipts.into_iter())
            .enumerate()
            .map(|(index, tx_rx)| {
                TxRx::try_create(
                    BlockItemIdentifier::new(number, index as u64),
                    tx_rx.0,
                    tx_rx.1,
                )
            })
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
    pub fn hash(&self) -> Option<BlockHash> {
        Some(self.hash)
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

pub(crate) type ExeFiller = JoinFill<
    Identity,
    JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
>;

#[derive(Debug, Clone)]
pub struct Client {
    url: Url,
    private_key: Option<String>,
    // ws: RootProvider<PubSubFrontend>,
    http: AlloyProvider,
    // what chain id is implied here? Maybe need to define internal chain ids for different attestation chains
    // and not rely on ethereum chain ids?
    chain_id: u64,
}

impl Client {
    pub async fn new(url: &str, private_key: Option<&str>) -> Result<Self> {
        let url = Url::parse(url)?;

        let http = ProviderBuilder::new()
            .network::<Ethereum>()
            .on_http(url.clone());

        info!("Connecting to Ethereum node at {}", url);

        let chain_id = http.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId
        })?;

        Ok(Self {
            url,
            private_key: private_key.map(|s| s.to_owned()),
            http,
            chain_id,
        })
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub async fn renew_http(&mut self) -> Result<()> {
        let http = ProviderBuilder::new()
            .network::<Ethereum>()
            .on_http(self.url.clone());

        self.http = http;
        Ok(())
    }

    #[must_use]
    pub fn get_url(&self) -> Url {
        self.url.clone()
    }

    pub async fn get_ws(&self) -> Result<AlloyProvider> {
        let mut url = self.url.clone();

        if url.scheme() == "http" {
            url.set_scheme("ws").map_err(|_| {
                Error::ClientError(anyhow::anyhow!(
                    "Cannot open websocket connection to ethereum node"
                ))
            })?;
        } else {
            url.set_scheme("wss").map_err(|_| {
                Error::ClientError(anyhow::anyhow!(
                    "Cannot open websocket connection to ethereum node"
                ))
            })?;
        }

        let ws = WsConnect::new(url);
        let provider = ProviderBuilder::new()
            .network::<Ethereum>()
            .on_ws(ws)
            .await?;

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

    pub async fn get_block(&self, number: u64) -> Result<OrderedBlock, Error> {
        info!(
            "Getting block {:?}",
            BlockId::Number(BlockNumberOrTag::Number(number))
        );

        let get_eth_block_fut = self.get_eth_block(number);
        let get_eth_receipts_fut = self.get_receipts(number);

        let (block, receipts) =
            futures::future::try_join(get_eth_block_fut, get_eth_receipts_fut).await?;

        if block.transactions.len() != receipts.len() {
            return Err(Error::TransactionsReceiptsMismatch(number));
        }

        let transactions = block.transactions.into_transactions().collect::<Vec<_>>();

        OrderedBlock::try_create(
            self.chain_id,
            number,
            block.header.hash,
            transactions,
            receipts,
        )
        .map_err(Error::TransactionConversion)
    }

    pub async fn get_raw_block(&self, number: u64) -> Result<OrderedRawBlock, Error> {
        let get_eth_block_fut = self.get_eth_block(number);
        let get_eth_receipts_fut = self.get_receipts(number);

        let (block, receipts) =
            futures::future::try_join(get_eth_block_fut, get_eth_receipts_fut).await?;

        if block.transactions.len() != receipts.len() {
            return Err(Error::TransactionsReceiptsMismatch(number));
        }

        let transactions = block.transactions.into_transactions().collect::<Vec<_>>();

        Ok(OrderedRawBlock::new(
            Some(self.chain_id),
            number,
            block.header.hash,
            transactions,
            receipts,
        ))
    }

    async fn get_receipts(&self, number: u64) -> Result<Vec<TransactionReceipt>, Error> {
        self.http
            .get_block_receipts(BlockId::Number(BlockNumberOrTag::Number(number)))
            .await
            .map_err(|e| {
                error!("Failed to get receipts: {:?}", e);
                Error::FailedToGetReceipts(number)
            })?
            .ok_or(Error::FailedToGetBlock(number))
    }

    async fn get_eth_block(&self, number: u64) -> Result<Block, Error> {
        self.http
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
        Ok(self.http.get_block_number().await?)
    }

    pub async fn get_chain_id(&self) -> Result<u64, Error> {
        self.http.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId
        })
    }
}

pub fn starknet_pedersen_mmr(block: &OrderedBlock) -> StarknetPedersenMerkleTree {
    // Create abi's for all transactions
    let abis = block
        .items()
        .iter()
        .map(BlockItem::to_bytes)
        .collect::<Vec<Vec<u8>>>();

    StarknetPedersenMerkleTree::from(&abis[..])
}
