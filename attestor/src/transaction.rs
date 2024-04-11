use alloy::consensus::{ReceiptWithBloom, Signed};
use alloy::consensus::{TxEip1559, TxEip2930, TxLegacy};
use alloy::core::primitives::{Address, Log, U256};
use alloy::rpc::types::eth;

pub trait BlockItem: Sized {
    fn to_bytes(&self) -> Vec<u8>;

    fn chain_id(&self) -> u64;
    fn block_number(&self) -> U256;
    fn index(&self) -> u64;
    fn from(&self) -> Address;
    fn to(&self) -> Option<Address>;
}

#[derive(Debug, Clone, Default)]
pub struct Transaction(pub eth::Transaction);

impl BlockItem for Transaction {
    fn to_bytes(&self) -> Vec<u8> {
        match self.0.transaction_type {
            Some(0) => {
                let tx: Signed<TxLegacy> = self.0.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(1) => {
                let tx: Signed<TxEip1559> = self.0.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(2) => {
                let tx: Signed<TxEip2930> = self.0.clone().try_into().unwrap();
                alloy::rlp::encode(tx.into_parts().0)
            }
            Some(_) => Vec::new(),
            None => Vec::new(),
        }
    }

    fn chain_id(&self) -> u64 {
        self.0.chain_id.unwrap_or_default()
    }

    fn index(&self) -> u64 {
        self.0.transaction_index.unwrap_or_default()
    }

    fn block_number(&self) -> U256 {
        U256::saturating_from(self.0.block_number.unwrap_or_default())
    }

    fn from(&self) -> Address {
        self.0.from
    }

    fn to(&self) -> Option<Address> {
        self.0.to
    }
}

#[derive(Debug, Clone)]
pub struct Receipt(pub alloy::rpc::types::eth::TransactionReceipt);

impl BlockItem for Receipt {
    fn to_bytes(&self) -> Vec<u8> {
        let rwb = self.0.inner.as_receipt_with_bloom().unwrap();
        let receipt = rwb.receipt.clone();

        let mut new_receipt = alloy::consensus::Receipt::default();
        new_receipt.cumulative_gas_used = receipt.cumulative_gas_used;
        new_receipt.status = receipt.status;
        let logs = receipt
            .logs
            .into_iter()
            .map(|l| {
                let log = Log::new(l.address(), l.topics().to_vec(), l.data().data.clone());
                log.unwrap()
            })
            .collect::<Vec<Log>>();
        new_receipt.logs = logs;

        let rwb_new: ReceiptWithBloom<Log> = ReceiptWithBloom::new(new_receipt, rwb.logs_bloom);
        alloy::rlp::encode(&rwb_new)
    }

    fn chain_id(&self) -> u64 {
        0
    }

    fn index(&self) -> u64 {
        self.0.transaction_index
    }

    fn block_number(&self) -> U256 {
        U256::saturating_from(self.0.block_number.unwrap_or_default())
    }

    fn from(&self) -> Address {
        self.0.from
    }

    fn to(&self) -> Option<Address> {
        self.0.to
    }
}
