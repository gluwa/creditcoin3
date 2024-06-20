use anyhow::anyhow;
use ethereum_types::U256;
use prover_primitives::claim::ClaimIdentifier;
use serde::{Deserialize, Serialize};
use utils::block_item_traits::BlockItemIdentifier;
use utils::json_serializable::JsonSerializable;
use utils::utils::{try_parse_felt, try_parse_u64, try_parse_usize, u256_from_felts};
use utils::Felt;
use utils::StarknetPedersenMerkleProof;
//pub type Felt = starknet_crypto::FieldElement;

#[derive(Serialize)]
pub struct ClaimDigestRoots {
    tx_root: String,
    rx_root: String,
}

impl ClaimDigestRoots {
    pub fn new(tx_root: &Felt, rx_root: &Felt) -> Self {
        Self {
            tx_root: tx_root.to_string(),
            rx_root: rx_root.to_string(),
        }
    }
}

#[allow(dead_code)]
pub fn felt_from_dec_str(s: &str) -> anyhow::Result<Felt> {
    match Felt::from_dec_str(s) {
        Ok(x) => Ok(x),
        Err(_) if s.starts_with('-') => {
            let neg_x = Felt::from_dec_str(&s[1..]).map_err(|err| anyhow!("{}", err))?;
            Ok(Felt::ZERO - neg_x)
        }
        Err(err) => Err(anyhow!("{}", err)),
    }
}

#[derive(Debug, Clone)]
pub struct CairoVerifierOutput {
    pub claim_id: ClaimIdentifier,
    pub continuity_checkpoint_digest: Felt,
    pub continuity_checkpoint_block_number: U256,
    pub query_hash: Felt,
    pub claim_fields: Vec<Felt>,
}

impl CairoVerifierOutput {
    const PREFIX: &'static str = "Program output:";

    pub fn try_from_prefixed_str(s: &str) -> Result<Self, String> {
        if s.len() < Self::PREFIX.len() {
            return Err("failed to parse output string".to_owned());
        }

        let mut prefix_index = None;
        for i in 0..(s.len() - Self::PREFIX.len()) {
            if &s[i..i + Self::PREFIX.len()] == Self::PREFIX {
                prefix_index = Some(i + Self::PREFIX.len());
                break;
            }
        }

        let prefix_index = prefix_index.ok_or(format!(
            "failed to parse output string. Expected to find '{}' prefix",
            Self::PREFIX
        ))?;

        Self::try_from(&s[prefix_index..].split_whitespace().collect::<Vec<_>>()[..])
    }

    fn parse_field<T, E: std::fmt::Debug, F: FnOnce(&str) -> Result<T, E>>(
        s: Option<&&str>,
        f: F,
        field_name: &str,
    ) -> Result<T, String> {
        s.ok_or("value for '{field_name}' is absent".to_owned())
            .and_then(|s| f(s).map_err(|err| format!("failed to parse '{field_name}': {err:?}")))
    }
}

impl TryFrom<&[&str]> for CairoVerifierOutput {
    type Error = String;

    fn try_from(ss: &[&str]) -> Result<Self, Self::Error> {
        let rlp_len = Self::parse_field(ss.last(), try_parse_usize, "rlp_len")?;

        let mut it = ss.iter();

        let claim_kind = (Self::parse_field(it.next(), try_parse_usize, "claim_kind")? as u8)
            .try_into()
            .map_err(|x| format!("invalid claim kind: {x}"))?;
        let block_number_lo = Self::parse_field(it.next(), try_parse_felt, "block_number_lo")?;
        let block_number_hi = Self::parse_field(it.next(), try_parse_felt, "block_number_hi")?;
        let claim_id = ClaimIdentifier {
            kind: claim_kind,
            block_item_id: BlockItemIdentifier::new(
                u256_from_felts(&block_number_lo, &block_number_hi),
                Self::parse_field(it.next(), try_parse_u64, "index")?,
            ),
        };
        //        let tx_type = Self::parse_field(it.next(), try_parse_u64, "tx_type")? as u8;
        let continuity_checkpoint_digest =
            Self::parse_field(it.next(), try_parse_felt, "continuity_checkpoint_digest")?;
        let continuity_checkpoint_block_number_lo = Self::parse_field(
            it.next(),
            try_parse_felt,
            "continuity_checkpoint_block_number_lo",
        )?;
        let continuity_checkpoint_block_number_hi = Self::parse_field(
            it.next(),
            try_parse_felt,
            "continuity_checkpoint_block_number_hi",
        )?;
        let continuity_checkpoint_block_number = u256_from_felts(
            &continuity_checkpoint_block_number_lo,
            &continuity_checkpoint_block_number_hi,
        );
        let query_hash = Self::parse_field(it.next(), try_parse_felt, "query_hash")?;
        it.take(rlp_len)
            .enumerate()
            .map(|(i, s)| Self::parse_field(Some(s), try_parse_felt, &format!("felt[{i}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(|claim_fields| Self {
                claim_id,
                //                tx_type,
                continuity_checkpoint_digest,
                continuity_checkpoint_block_number,
                query_hash,
                claim_fields,
            })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MerkleProofSerializable {
    height: usize,
    arity: usize,
    root: String,
    path: Vec<Vec<String>>,
    claim_rlp: Vec<u8>,
    leaf_hash_prefix: u8,
    inner_node_hash_prefix: u8,
}

impl From<(StarknetPedersenMerkleProof, Vec<u8>)> for MerkleProofSerializable {
    fn from((proof, claim_rlp): (StarknetPedersenMerkleProof, Vec<u8>)) -> Self {
        Self {
            height: proof.height(),
            arity: StarknetPedersenMerkleProof::arity(),
            root: proof.root().0.to_string(),
            path: proof
                .path()
                .as_ref()
                .iter()
                .map(|item| {
                    let mut v: Vec<_> = item
                        .hashes()
                        .iter()
                        .map(|felt_wrapped| felt_wrapped.0.to_string())
                        .collect();
                    v.push(item.offset().to_string());
                    v
                })
                .collect(),
            claim_rlp,
            leaf_hash_prefix: mmr::LEAF_HASH_PREPEND_VALUE,
            inner_node_hash_prefix: mmr::INNER_HASH_PREPEND_VALUE,
        }
    }
}

#[derive(Debug)]
pub enum ClaimProverError {
    AttestationFragmentMismatch(u64, Option<u64>, Option<u64>),
    SerializationFailure(String),
    BlockFetchFailure(String),
    ClaimNotIdentified,
    ClaimNotUnique,
    InputFileNameNotSet,
    OutputFileNameNotSet,
    OutputParseFailure(String),
    Cairo(ScriptError),
    TempDirPathConversionError(String),
    Other(String),
}

impl From<ScriptError> for ClaimProverError {
    fn from(err: ScriptError) -> Self {
        Self::Cairo(err)
    }
}

#[derive(Debug)]
pub enum ScriptError {
    BadArgs,
    InputFiles(i32),
    Compilation(i32),
    Run(i32),
    StoneProver(i32),
    StoneVerifier(i32),
    AttestationProgramCompilation(i32),
    Other(i32),
    ProcessExecutionFailure,
    Unspecified,
}

impl From<Option<i32>> for ScriptError {
    fn from(code: Option<i32>) -> Self {
        if let Some(code) = code {
            match code {
                10..=19 => Self::BadArgs,
                20..=29 => Self::InputFiles(code),
                30..=39 => Self::Compilation(code),
                40..=49 => Self::Run(code),
                50..=59 => Self::StoneProver(code),
                60..=69 => Self::StoneVerifier(code),
                70 => Self::AttestationProgramCompilation(code),
                _ => Self::Other(code),
            }
        } else {
            Self::Unspecified
        }
    }
}

pub type StoneProofPublicInput = crate::CairoVerifierOutput;

impl StoneProofPublicInput {
    const NUMBER_OF_STATIC_FIELDS: usize = 4 + 1 + 1 + 2;
}

impl TryFrom<&StoneProofJson> for StoneProofPublicInput {
    type Error = String;

    fn try_from(proof: &StoneProofJson) -> Result<Self, String> {
        let public_memory = &proof.public_input.public_memory.0;
        let rlp_len = public_memory
            .last()
            .ok_or("proof public input is empty".to_owned())
            .map(PublicMemoryItem::value)
            .and_then(|s| {
                try_parse_usize(s).map_err(|err| format!("failed to parse 'rlp_len': {err:?}"))
            })?;

        Self::try_from(
            &public_memory
                .iter()
                .rev()
                .take(rlp_len + Self::NUMBER_OF_STATIC_FIELDS + 1)
                .rev()
                .map(PublicMemoryItem::value)
                .collect::<Vec<_>>()[..],
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicMemoryItem {
    address: u64,
    page: u64,
    value: Box<str>,
}

impl PublicMemoryItem {
    pub fn value(&self) -> &str {
        self.value.as_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicMemory(Vec<PublicMemoryItem>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicInput {
    pub public_memory: PublicMemory,
    dynamic_params: serde_json::value::Value,
    layout: serde_json::value::Value,
    memory_segments: serde_json::value::Value,
    n_steps: serde_json::value::Value,
    rc_max: serde_json::value::Value,
    rc_min: serde_json::value::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoneProofJson {
    pub proof_hex: Box<str>,
    pub public_input: PublicInput,
    pub proof_parameters: serde_json::value::Value,
    pub annotations: serde_json::value::Value,
    pub prover_config: serde_json::value::Value,
    pub private_input: serde_json::value::Value,
}

impl JsonSerializable for StoneProofJson {}

#[derive(Serialize, Deserialize, Debug)]
pub struct StoneProof(StoneProofJson);

impl StoneProof {
    pub fn proof(&self) -> &StoneProofJson {
        &self.0
    }
    pub fn strip_off_annotations(&mut self) -> &mut Self {
        self.proof_mut().annotations = serde_json::value::Value::Null;
        self
    }
    pub fn strip_off_prover_config(&mut self) -> &mut Self {
        self.proof_mut().prover_config = serde_json::value::Value::Null;
        self
    }
    pub fn strip_off_private_input(&mut self) -> &mut Self {
        self.proof_mut().private_input = serde_json::value::Value::Null;
        self
    }

    fn proof_mut(&mut self) -> &mut StoneProofJson {
        &mut self.0
    }
}

impl From<StoneProofJson> for StoneProof {
    fn from(proof_json: StoneProofJson) -> Self {
        Self(proof_json)
    }
}

impl JsonSerializable for StoneProof {}
