use alloy::primitives::BlockHash;
use alloy::rpc::types::{Transaction, TransactionReceipt};
use serde::{Deserialize, Serialize};

pub trait CacheT<T>: Clone {
    type CachedItem: TryInto<T> + serde::Serialize + for<'a> serde::Deserialize<'a>;

    fn key(&self) -> &str;
    fn try_create_key(&mut self) -> anyhow::Result<()>;

    fn try_read(&self) -> anyhow::Result<Self::CachedItem> {
        let file = std::fs::File::open(self.key())?;
        Ok(serde_json::from_reader::<_, Self::CachedItem>(file)?)
    }

    fn try_write(&mut self, item: &Self::CachedItem) -> anyhow::Result<()> {
        self.try_create_key()?;

        let file = std::fs::File::create(self.key())?;

        Ok(serde_json::to_writer_pretty(file, item)?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderedBlockJson {
    pub chain_id: Option<u64>,
    pub number: u64,
    pub hash: Option<BlockHash>,
    pub items: Vec<(Transaction, TransactionReceipt)>,
}

#[derive(Clone)]
pub struct BlockCache {
    url: String,
    dir: String,
}

impl BlockCache {
    pub fn new(dir: &str, block_number: u64) -> Self {
        Self {
            url: dir.to_owned() + "/block_" + &format!("{block_number}") + ".json",
            dir: dir.to_owned(),
        }
    }
}

impl CacheT<OrderedBlockJson> for BlockCache {
    type CachedItem = OrderedBlockJson;

    fn key(&self) -> &str {
        &self.url
    }

    fn try_create_key(&mut self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(std::fs::File::create(&self.url).map(|_| ())?)
    }
}
