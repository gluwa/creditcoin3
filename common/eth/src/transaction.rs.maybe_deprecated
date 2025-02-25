use alloy::consensus::{ReceiptWithBloom, Signed};
use alloy::consensus::{TxEip1559, TxEip2930, TxEip4844, TxLegacy};
use alloy::core::primitives::Log;
use utils::{block_item_traits::BlockItemIdentifier, StarknetPedersenMerkleTree};

use crate::{AlloyTransaction, AlloyTransactionReceipt};

pub trait BlockItem: Sized {
    fn to_bytes(&self) -> Vec<u8>;

    fn id(&self) -> &BlockItemIdentifier;
    fn tx_type(&self) -> Option<u8>;
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub inner: alloy::rpc::types::eth::Transaction,
    id: BlockItemIdentifier,
}

impl Transaction {
    pub fn payload_bytes(&self) -> Vec<u8> {
        match self.tx_type() {
            Some(0) => {
                let tx: Signed<TxLegacy> = self.inner.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(1) => {
                let tx: Signed<TxEip2930> = self.inner.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(2) => {
                let tx: Signed<TxEip1559> = self.inner.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(3) => {
                let tx: Signed<TxEip4844> = self.inner.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            _ => unimplemented!("unsupported tx type"),
        }
    }
}

impl Transaction {
    pub fn new(tx: AlloyTransaction, id: BlockItemIdentifier) -> Self {
        Self { inner: tx, id }
    }
}

impl BlockItem for Transaction {
    fn to_bytes(&self) -> Vec<u8> {
        let tx_id = self.id().to_bytes();
        let tx_rlp = self.payload_bytes();

        let mut bytes = Vec::with_capacity(tx_id.len() + tx_rlp.len() + 1);

        bytes.extend(tx_id);
        bytes.extend(tx_rlp);

        bytes
    }

    fn id(&self) -> &BlockItemIdentifier {
        &self.id
    }

    fn tx_type(&self) -> Option<u8> {
        self.inner.transaction_type
    }
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub inner: AlloyTransactionReceipt,
    id: BlockItemIdentifier,
}

impl Receipt {
    pub fn new(rx: AlloyTransactionReceipt, id: BlockItemIdentifier) -> Self {
        Self { inner: rx, id }
    }
}

impl BlockItem for Receipt {
    fn to_bytes(&self) -> Vec<u8> {
        let rwb = self.inner.inner.as_receipt_with_bloom().unwrap();

        let receipt = rwb.receipt.clone();

        let logs = receipt
            .logs
            .into_iter()
            .map(|l| {
                let log = Log::new(l.address(), l.topics().to_vec(), l.data().data.clone());
                log.unwrap()
            })
            .collect::<Vec<Log>>();

        let new_receipt = alloy::consensus::Receipt {
            cumulative_gas_used: receipt.cumulative_gas_used,
            status: receipt.status,
            logs,
        };

        let rwb_new: ReceiptWithBloom<Log> = ReceiptWithBloom::new(new_receipt, rwb.logs_bloom);
        alloy::rlp::encode(&rwb_new)
    }

    fn id(&self) -> &BlockItemIdentifier {
        &self.id
    }

    fn tx_type(&self) -> Option<u8> {
        if 0 == self.inner.transaction_type() as u8 {
            None
        } else {
            Some(self.inner.transaction_type() as u8)
        }
    }
}


pub fn starknet_pedersen_mmr(
    transactions: Vec<Transaction>,
    receipts: Vec<Receipt>,
) -> (StarknetPedersenMerkleTree, StarknetPedersenMerkleTree) {
    // Create rlp's for all transactions
    let tx_rlps = transactions
        .iter()
        .map(BlockItem::to_bytes)
        .collect::<Vec<Vec<u8>>>();

    let rx_rlps = receipts
        .iter()
        .map(BlockItem::to_bytes)
        .collect::<Vec<Vec<u8>>>();

    let tx_tree = StarknetPedersenMerkleTree::from(&tx_rlps[..]);
    let rx_tree = StarknetPedersenMerkleTree::from(&rx_rlps[..]);

    (tx_tree, rx_tree)
}
