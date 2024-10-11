use crate::types::StoneProof;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::collections::HashMap;
use utils::json_serializable::JsonSerializable;

pub type StarkProgramAuthHash = H256;

// Version 1 hash
pub const STARK_PROGRAM_V1_HASH: StarkProgramAuthHash = H256([
    231, 189, 205, 230, 13, 221, 69, 124, 167, 243, 68, 105, 63, 104, 245, 56, 126, 209, 169, 222,
    112, 132, 191, 163, 100, 141, 104, 195, 2, 102, 226, 196,
]);

// Version 2 hash
pub const STARK_PROGRAM_V2_HASH: StarkProgramAuthHash = H256([
    232, 88, 85, 136, 197, 79, 34, 49, 253, 15, 116, 194, 99, 235, 158, 244, 247, 191, 215, 123,
    22, 67, 23, 250, 78, 242, 36, 224, 60, 55, 37, 201,
]);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StarkProgramMetadata {
    pub version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarkProgramMetadataStorage {
    pub map: HashMap<StarkProgramAuthHash, StarkProgramMetadata>,
    pub last_version: u8,
}

impl JsonSerializable for StarkProgramMetadataStorage {}

impl StarkProgramMetadataStorage {
    pub const DEFAULT_URL: &'static str = "stark_program_metadata.json";

    pub fn try_append(
        &mut self,
        key: StarkProgramAuthHash,
        metadata: StarkProgramMetadata,
    ) -> anyhow::Result<()> {
        if self.last_version < metadata.version {
            self.last_version = metadata.version;
            self.map.insert(key, metadata);
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "metadata version > {} expected",
                self.last_version
            ))
        }
    }

    pub fn store_onchain_sim(&self, url: &str) -> anyhow::Result<()> {
        self.to_file(url)
    }

    pub fn retrieve_from_chain_sim(url: &str) -> anyhow::Result<Self> {
        Self::try_from_file(url)
    }

    pub fn metadata(&self, h: &StarkProgramAuthHash) -> Option<&StarkProgramMetadata> {
        self.map.get(h)
    }
}

impl Default for StarkProgramMetadataStorage {
    fn default() -> Self {
        let mut map = HashMap::default();
        map.insert(STARK_PROGRAM_V1_HASH, StarkProgramMetadata { version: 1 });
        map.insert(STARK_PROGRAM_V2_HASH, StarkProgramMetadata { version: 2 });

        Self {
            map,
            last_version: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub enum StarkProgramAuthError {
    AuthenticationFailure(StarkProgramAuthHash),
    Other(String),
}
pub struct StarkProgramAuth;

impl StarkProgramAuth {
    pub fn authenticate<'a>(
        proof: &StoneProof,
        metadata_storage: &'a StarkProgramMetadataStorage,
        hasher: impl FnOnce(&[u8]) -> StarkProgramAuthHash,
    ) -> Result<&'a StarkProgramMetadata, StarkProgramAuthError> {
        let h = hasher(
            &proof
                .program_bytes()
                .map_err(StarkProgramAuthError::Other)?[..],
        );

        metadata_storage
            .map
            .get(&h)
            .ok_or(StarkProgramAuthError::AuthenticationFailure(h))
    }
}

#[cfg(test)]
mod tests {
    use sp_core::H256;

    use super::StarkProgramMetadataStorage;
    use crate::stark_program_auth::StarkProgramMetadata;

    #[test]
    fn append_metadata_test() {
        let mut map = StarkProgramMetadataStorage::default();

        let random_hash = H256::random();

        map.try_append(random_hash, StarkProgramMetadata { version: 4 })
            .unwrap();
        assert_eq!(
            map.metadata(&random_hash),
            Some(&StarkProgramMetadata { version: 4 })
        );

        assert!(map
            .try_append(random_hash, StarkProgramMetadata { version: 2 })
            .is_err());
    }
}
