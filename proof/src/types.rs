use anyhow::anyhow;
use attestor::merkle::tree::StarknetPedersenMerkleProof;
use prover_primitives::claim::ClaimKind;
use serde::{Deserialize, Serialize};

pub type Felt = starknet_crypto::FieldElement;

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

#[derive(Debug)]
pub struct CairoVerifierOutput {
    pub claim_proof_root: Felt,
    pub claim_kind: ClaimKind,
    pub claim_index: u64,
    pub block_number: u64,
    pub chain_id: u8,
    pub claim_from: Felt,
    pub claim_to: Felt,
    pub continuity_checkpoint_digest: Felt,
    pub continuity_checkpoint_block_number: u64,
}

impl CairoVerifierOutput {
    const PREFIX: &'static str = "Program output:";
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

impl TryFrom<&str> for CairoVerifierOutput {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, String> {
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

        if let Some(prefix_index) = prefix_index {
            let mut values_iter = s[prefix_index..].split_whitespace();

            Ok(Self {
                claim_proof_root: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_proof_root' is absent".to_owned())?,
                )
                .map_err(|err| format!("failed to parse 'claim_proof_root': {err:?}"))?,

                claim_kind: values_iter
                    .next()
                    .ok_or("value for 'claim_kind' is absent".to_owned())?
                    .parse::<u8>()
                    .map_err(|err| format!("failed to parse 'claim_kind': {err:?}"))?
                    .try_into()
                    .map_err(|err| format!("'claim_kind': {err:?}"))?,

                claim_index: values_iter
                    .next()
                    .ok_or("value for 'claim_index' is absent".to_owned())?
                    .parse::<u64>()
                    .map_err(|err| format!("failed to parse 'claim_index': {err:?}"))?,
                block_number: values_iter
                    .next()
                    .ok_or("value for 'block_number' is absent".to_owned())?
                    .parse::<u64>()
                    .map_err(|err| format!("failed to parse 'block_number': {err:?}"))?,
                chain_id: values_iter
                    .next()
                    .ok_or("value for 'chain_id' is absent".to_owned())?
                    .parse::<u8>()
                    .map_err(|err| format!("failed to parse 'chain_id': {err:?}"))?,
                claim_from: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_from' is absent".to_owned())?,
                )
                .map_err(|err| format!("failed to parse 'claim_from': {err:?}"))?,
                claim_to: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'claim_to' is absent".to_owned())?,
                )
                .map_err(|err| format!("failed to parse 'claim_to': {err:?}"))?,
                continuity_checkpoint_digest: felt_from_dec_str(
                    values_iter
                        .next()
                        .ok_or("value for 'continuity_checkpoint_digest' is absent".to_owned())?,
                )
                .map_err(|err| {
                    format!("failed to parse 'continuity_checkpoint_digest': {err:?}")
                })?,
                continuity_checkpoint_block_number: values_iter
                    .next()
                    .ok_or("value for 'continuity_checkpoint_block_number' is absent")?
                    .parse::<u64>()
                    .map_err(|err| {
                        format!("failed to parse 'continuity_checkpoint_block_number': {err:?}")
                    })?,
            })
        } else {
            Err(format!(
                "failed to parse output string. Expected to find '{}' prefix",
                Self::PREFIX
            ))
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MerkleProofWithClaimJson {
    height: usize,
    arity: usize,
    root: String,
    path: Vec<Vec<String>>,
    claim_rlp: Vec<u8>,
    leaf_hash_prefix: u8,
    inner_node_hash_prefix: u8,
    claim_kind: ClaimKind,
}

impl From<(StarknetPedersenMerkleProof, Vec<u8>, ClaimKind)> for MerkleProofWithClaimJson {
    fn from(
        (proof, claim_rlp, claim_kind): (StarknetPedersenMerkleProof, Vec<u8>, ClaimKind),
    ) -> Self {
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
            claim_kind,
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
