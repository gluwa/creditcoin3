use alloy::core::primitives::Log;
use alloy::{
    consensus::{
        ReceiptWithBloom, SignableTransaction, Signed, TxEip1559, TxEip2930, TxEip4844, TxLegacy,
        TxReceipt,
    },
    primitives::BlockHash,
    providers::{network::TransactionResponse, Provider, ProviderBuilder, RootProvider},
    pubsub::PubSubFrontend,
    rlp::{BufMut, Encodable},
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
}

#[derive(Debug)]
pub enum TypedTransaction {
    Legacy(Signed<TxLegacy>, BlockHash),
    Type1(Signed<TxEip2930>, BlockHash),
    Type2(Signed<TxEip1559>, BlockHash),
    Type3(Signed<TxEip4844>, BlockHash),
}

impl TypedTransaction {
    pub fn tx_type(&self) -> Option<u8> {
        match self {
            Self::Legacy(_, _) => None,
            Self::Type1(_, _) => Some(1),
            Self::Type2(_, _) => Some(2),
            Self::Type3(_, _) => Some(3),
        }
    }

    pub fn tx_hash(&self) -> &BlockHash {
        match self {
            Self::Legacy(_, h) => h,
            Self::Type1(_, h) => h,
            Self::Type2(_, h) => h,
            Self::Type3(_, h) => h,
        }
    }

    fn fields_len(&self) -> usize {
        match self {
            Self::Legacy(tx, _) => {
                tx.tx().fields_len() + tx.signature().length() + tx.tx().signature_hash().length()
            }
            Self::Type1(tx, _) => {
                tx.tx().fields_len() + tx.signature().length() + tx.tx().signature_hash().length()
            }
            Self::Type2(tx, _) => {
                tx.tx().fields_len() + tx.signature().length() + tx.tx().signature_hash().length()
            }
            Self::Type3(tx, _) => {
                tx.tx().fields_len() + tx.signature().length() + tx.tx().signature_hash().length()
            }
        }
    }
}

impl TryFrom<Transaction> for TypedTransaction {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> std::result::Result<Self, Self::Error> {
        let tx_hash = tx.tx_hash();

        Ok(match tx.transaction_type {
            None | Some(0) => Self::Legacy(tx.try_into()?, tx_hash),
            Some(1) => Self::Type1(tx.try_into()?, tx_hash),
            Some(2) => Self::Type2(tx.try_into()?, tx_hash),
            Some(3) => Self::Type3(tx.try_into()?, tx_hash),
            t => unimplemented!("transaction type not supported: {}", t.unwrap()),
        })
    }
}

impl Encodable for TypedTransaction {
    fn encode(&self, out: &mut dyn BufMut) {
        match self {
            Self::Legacy(tx, _) => {
                tx.tx().nonce.encode(out);
                tx.tx().gas_price.encode(out);
                tx.tx().gas_limit.encode(out);
                tx.tx().to.encode(out);
                tx.tx().value.encode(out);
                tx.tx().input.0.encode(out);
                tx.signature().encode(out);
                tx.tx().signature_hash().encode(out);
            }

            Self::Type1(tx, _) => {
                tx.tx().chain_id.encode(out);
                tx.tx().nonce.encode(out);
                tx.tx().gas_price.encode(out);
                tx.tx().gas_limit.encode(out);
                tx.tx().to.encode(out);
                tx.tx().value.encode(out);
                tx.tx().input.0.encode(out);
                tx.tx().access_list.encode(out);
                tx.signature().encode(out);
                tx.tx().signature_hash().encode(out);
            }

            Self::Type2(tx, _) => {
                tx.tx().chain_id.encode(out);
                tx.tx().nonce.encode(out);
                tx.tx().max_fee_per_gas.encode(out);
                tx.tx().max_priority_fee_per_gas.encode(out);
                tx.tx().gas_limit.encode(out);
                tx.tx().to.encode(out);
                tx.tx().value.encode(out);
                tx.tx().input.0.encode(out);
                tx.tx().access_list.encode(out);
                tx.signature().encode(out);
                tx.tx().signature_hash().encode(out);
            }
            Self::Type3(tx, _) => {
                tx.tx().chain_id.encode(out);
                tx.tx().nonce.encode(out);
                tx.tx().gas_limit.encode(out);
                tx.tx().max_fee_per_gas.encode(out);
                tx.tx().max_priority_fee_per_gas.encode(out);
                tx.tx().to.encode(out);
                tx.tx().value.encode(out);
                tx.tx().input.0.encode(out);
                tx.tx().access_list.encode(out);
                tx.tx().blob_versioned_hashes.encode(out);
                tx.tx().max_fee_per_blob_gas.encode(out);
                tx.signature().encode(out);
                tx.tx().signature_hash().encode(out);
            }
        }
    }
}

#[derive(Debug)]
pub struct TxRx {
    id: BlockItemIdentifier,
    tx: TypedTransaction,
    rx: ReceiptWithBloom<Log>,
}

impl Encodable for TxRx {
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_length = self.payload_len();

        alloy::rlp::Header {
            list: true,
            payload_length,
        }
        .encode(out);
        self.tx.encode(out);

        self.rx.status_or_post_state().encode(out);
        self.rx.cumulative_gas_used().encode(out);
        self.rx.bloom().encode(out);
        self.rx.logs().to_vec().encode(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.payload_len();
        alloy::rlp::Header {
            list: true,
            payload_length,
        }
        .length()
            + payload_length
    }
}

impl TxRx {
    pub fn try_create(
        id: BlockItemIdentifier,
        tx: Transaction,
        rx: TransactionReceipt,
    ) -> Result<Self, ConversionError> {
        Ok(Self {
            id,
            tx: tx.try_into()?,
            rx: Self::transform_rx(rx).ok_or(ConversionError::Custom(
                "Receipt to ReceiptWithBloom conversion failed".to_owned(),
            ))?,
        })
    }

    pub fn tx(&self) -> &TypedTransaction {
        &self.tx
    }
    pub fn rx(&self) -> &ReceiptWithBloom<Log> {
        &self.rx
    }

    pub fn tx_hash(&self) -> &BlockHash {
        self.tx.tx_hash()
    }

    fn payload_len(&self) -> usize {
        let tx_fields_len = self.tx.fields_len();

        let rx_fields_len = self.rx.status_or_post_state().length()
            + self.rx.cumulative_gas_used().length()
            + self.rx.bloom().length()
            + self.rx.logs().to_vec().length();

        tx_fields_len + rx_fields_len
    }

    fn transform_rx(rx: TransactionReceipt) -> Option<ReceiptWithBloom<Log>> {
        let rwb = rx.inner.as_receipt_with_bloom()?;
        rwb.receipt
            .logs
            .iter()
            .map(|l| Log::new(l.address(), l.topics().to_vec(), l.data().data.clone()))
            .collect::<Option<Vec<Log>>>()
            .map(|logs| {
                let new_receipt = alloy::consensus::Receipt {
                    cumulative_gas_used: rwb.receipt.cumulative_gas_used,
                    status: rwb.receipt.status,
                    logs,
                };

                ReceiptWithBloom::new(new_receipt, rwb.logs_bloom)
            })
    }
}

impl BlockItem for TxRx {
    fn payload_bytes(&self) -> Vec<u8> {
        alloy::rlp::encode(self)
    }

    fn id(&self) -> &BlockItemIdentifier {
        &self.id
    }

    fn tx_type(&self) -> Option<u8> {
        self.tx.tx_type()
    }
}

#[derive(Debug)]
pub struct OrderedBlock {
    chain_id: u64,
    number: u64,
    hash: Option<BlockHash>,
    items: Vec<TxRx>,
}

impl OrderedBlock {
    pub fn try_create(
        chain_id: u64,
        number: u64,
        hash: Option<BlockHash>,
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
        self.hash
    }
    pub fn items(&self) -> &[TxRx] {
        &self.items[..]
    }
}

pub struct OrderedRawBlock {
    pub chain_id: Option<u64>,
    pub number: u64,
    pub hash: Option<BlockHash>,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
}

impl OrderedRawBlock {
    pub fn new(
        chain_id: Option<u64>,
        number: u64,
        hash: Option<BlockHash>,
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

#[derive(Debug, Clone)]
pub struct Client {
    url: Url,
    private_key: String,
    // ws: RootProvider<PubSubFrontend>,
    http: RootProvider<alloy::transports::http::Http<alloy::transports::http::Client>>,
    // what chain id is implied here? Maybe need to define internal chain ids for different attestation chains
    // and not rely on ethereum chain ids?
    chain_id: u64,
}

impl Client {
    pub async fn new(
        url: impl Into<String> + Copy,
        private_key: impl Into<String> + Copy,
    ) -> Result<Self> {
        let url = Url::parse(&url.into())?;

        let http = ProviderBuilder::new().on_http(url.clone());
        info!("Connecting to Ethereum node at {}", url);

        let chain_id = http.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId
        })?;

        Ok(Self {
            url,
            private_key: private_key.into(),
            http,
            chain_id,
        })
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub async fn renew_http(&mut self) -> Result<()> {
        let http = ProviderBuilder::new().on_http(self.url.clone());
        self.http = http;
        Ok(())
    }

    #[must_use]
    pub fn get_url(&self) -> Url {
        self.url.clone()
    }

    pub async fn get_ws(&self) -> Result<RootProvider<PubSubFrontend>> {
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
        let provider = ProviderBuilder::new().on_ws(ws).await?;

        Ok(provider)
    }

    pub fn get_signer(&self) -> Result<PrivateKeySigner> {
        let decoded = hex::decode(self.private_key.clone().replace("0x", ""))?;
        let signing_key = SigningKey::from_slice(&decoded)?;

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
            .get_block_receipts(BlockNumberOrTag::Number(number))
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
    // Create rlp's for all transactions
    let rlps = block
        .items()
        .iter()
        .map(BlockItem::to_bytes)
        .collect::<Vec<Vec<u8>>>();

    StarknetPedersenMerkleTree::from(&rlps[..])
}
