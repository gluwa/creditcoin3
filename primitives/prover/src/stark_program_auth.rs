use crate::types::StoneProof;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utils::json_serializable::JsonSerializable;

pub type StarkProgramAuthHash = u64;

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

    const V1_DEV: u8 = 1;
    const AUTH_HASH_V1_DEV: StarkProgramAuthHash = 18171554912147335677;
    const V2_DEV: u8 = 2;
    const AUTH_HASH_V2_DEV: StarkProgramAuthHash = 3438002004860300627;
    const V3_DEV: u8 = 3;
    const AUTH_HASH_V3_DEV: StarkProgramAuthHash = 617734937651202173;

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
        map.insert(
            Self::AUTH_HASH_V1_DEV,
            StarkProgramMetadata {
                version: Self::V1_DEV,
            },
        );
        map.insert(
            Self::AUTH_HASH_V2_DEV,
            StarkProgramMetadata {
                version: Self::V2_DEV,
            },
        );
        map.insert(
            Self::AUTH_HASH_V3_DEV,
            StarkProgramMetadata {
                version: Self::V3_DEV,
            },
        );

        Self {
            map,
            last_version: Self::V3_DEV,
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
    use super::StarkProgramMetadataStorage;
    use crate::stark_program_auth::StarkProgramMetadata;

    #[test]
    fn append_metadata_test() {
        let mut map = StarkProgramMetadataStorage::default();
        map.try_append(42, StarkProgramMetadata { version: 4 })
            .unwrap();
        assert_eq!(
            map.metadata(&42),
            Some(&StarkProgramMetadata { version: 4 })
        );

        assert!(map
            .try_append(42, StarkProgramMetadata { version: 2 })
            .is_err());
    }
}
