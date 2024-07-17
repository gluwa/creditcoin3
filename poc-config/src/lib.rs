use utils::json_serializable::JsonSerializable;
use serde::{Deserialize, Serialize};

const DEFAULT_CONFIG_FILE: &str = "../config.json";
const DEFAULT_ATTESTATION_CHAIN_DIR: &str = "attestation-chain";
const DEFAULT_ATTESTATION_CHAIN_FILE: &str = "chain.json";

const DATA_ROOT_DIR: &str = "../data";
const EXECUTION_CHAIN_DIR: &str = "execution-chain";
const ATTESTATION_DB_DIR: &str = "db";


#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PocConfig {
    source_chain_api_server_url: String,
    block_cache_url: Option<String>,
    execution_chain_url: Option<String>,

    attestation_blocks_builder: Option<AttestationBlocksBuilderConfig>,
    demo_prover: Option<DemoProverConfig>,
    claim_prover: Option<ClaimProverConfig>,
}

impl JsonSerializable for PocConfig {}

impl PocConfig {
    pub fn try_default() -> Result<Self, anyhow::Error> {
        Self::try_from_file(DEFAULT_CONFIG_FILE)
    }

    pub fn default_file() -> &'static str {
        DEFAULT_CONFIG_FILE
    }

    pub fn source_chain_api_server_url(&self) -> &str {
        &self.source_chain_api_server_url
    }

    pub fn block_cache_url(&self) -> Option<&str> {
        self.block_cache_url.as_deref()
    }

    pub fn attestation_blocks_builder(&self) -> Option<&AttestationBlocksBuilderConfig> {
        self.attestation_blocks_builder.as_ref()
    }
    pub fn demo_prover(&self) -> Option<&DemoProverConfig> {
        self.demo_prover.as_ref()
    }

    pub fn claim_prover(&self) -> Option<&ClaimProverConfig> {
        self.claim_prover.as_ref()
    }

    pub fn execution_chain_url(&self) -> Option<&str> {
        self.execution_chain_url.as_deref()
    }
    pub fn default_execution_chain_url() -> String {
        format!("{}/{}", crate::DATA_ROOT_DIR, crate::EXECUTION_CHAIN_DIR)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AttestationBlocksBuilderConfig {
    source_chain_wss_server_url: String,
    max_num_of_blocks_to_retrieve: Option<usize>,
    output_path: Option<String>,
    output_file: Option<String>,
}

impl JsonSerializable for AttestationBlocksBuilderConfig {}

impl AttestationBlocksBuilderConfig {
    const DEFAULT_MAX_BLOCKS_TO_RETRIEVE: usize = 5;
    pub fn source_chain_wss_server_url(&self) -> &str {
        &self.source_chain_wss_server_url
    }

    pub fn max_num_of_blocks_to_retrieve(&self) -> Option<usize> {
        self.max_num_of_blocks_to_retrieve
    }
    pub fn default_max_num_of_blocks_to_retrieve() -> usize {
        Self::DEFAULT_MAX_BLOCKS_TO_RETRIEVE
    }

    pub fn output_file(&self) -> Option<&str> {
        self.output_file.as_deref()
    }

    pub fn default_output_file() -> &'static str {
        DEFAULT_ATTESTATION_CHAIN_FILE
    }

    pub fn path(&self) -> String {
        match self.output_path.clone() {
            Some(path) => path,
            None => Self::default_path(),
        }
    }

    pub fn default_path() -> String {
        format!("{}/{}", crate::DATA_ROOT_DIR, DEFAULT_ATTESTATION_CHAIN_DIR)
    }

    pub fn full_path(&self) -> String {
        self.output_path
            .as_ref()
            .map(|path| {
                path.to_owned()
                    + "/"
                    + self
                        .output_file()
                        .unwrap_or_else(|| Self::default_output_file())
            })
            .unwrap_or_else(|| {
                Self::default_path()
                    + "/"
                    + self
                        .output_file()
                        .unwrap_or_else(|| Self::default_output_file())
            })
    }

    pub fn default_full_path() -> String {
        Self::default_path() + "/" + Self::default_output_file()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DemoProverConfig {
    db_url: Option<String>,
}

impl JsonSerializable for DemoProverConfig {}

impl DemoProverConfig {
    pub fn db_url(&self) -> Option<&str> {
        self.db_url.as_deref()
    }
    pub fn default_db_url() -> String {
        format!("{}/{}", crate::DATA_ROOT_DIR, crate::ATTESTATION_DB_DIR)
    }
}
impl Default for DemoProverConfig {
    fn default() -> Self {
        Self {
            db_url: Some(Self::default_db_url()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaimProverConfig {
    attestation_chain: Option<String>,
}

impl JsonSerializable for ClaimProverConfig {}

impl ClaimProverConfig {
    pub fn attestation_chain_file(&self) -> Option<String> {
        self.attestation_chain.clone()
    }

    pub fn default_attestation_chain_file() -> String {
        AttestationBlocksBuilderConfig::default_full_path()
    }
}
