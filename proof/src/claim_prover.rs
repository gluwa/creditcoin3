use serde::Serialize;
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};

use attestation_chain::attestation_fragment::{FragmentSlice, FragmentSliceSerializable};
use mmr::traits::MerkleTreeTrait;
use prover_primitives::claim::{ClaimKind, ClaimSerializable};
use utils::json_serializable::JsonSerializable;
use utils::{StarknetPedersenMerkleProof, StarknetPedersenMmr};

use crate::types::{
    CairoVerifierOutput, ClaimDigestRoots, ClaimProverError, MerkleProofSerializable, ScriptError,
    StoneProof, StoneProofJson,
};

use crate::json_serializable::JsonSerializable;

const DATA_ROOT_DIR: &str = "../data";
const CLAIM_PROOF_DIR: &str = "claim-proofs";

fn claim_proof_dir() -> String {
    format!("{}/{}", DATA_ROOT_DIR, CLAIM_PROOF_DIR)
}

#[derive(Serialize)]
pub struct ClaimProver<'a> {
    merkle_proof: MerkleProofSerializable,
    claim_digest_roots: ClaimDigestRoots,
    attestation_chain: FragmentSliceSerializable<'a>,
    claim: ClaimSerializable,

    #[serde(skip)]
    cairo_input_file: Option<String>,
    #[serde(skip)]
    cairo_output_file: Option<String>,
    #[serde(skip)]
    stone_proof_file: Option<String>,
    #[serde(skip)]
    dir: Option<String>,
    #[serde(skip)]
    cairo_output: Option<CairoVerifierOutput>,
}

impl<'a> ClaimProver<'a> {
    const SCRIPT_SOURCE: &'static str = "../cairo/scripts/verify_merkle_proof.sh";
    const STONE_PROVER_SCRIPT_SOURCE: &'static str = "../cairo/scripts/stone_prove_claim.sh";

    pub fn script_source() -> &'static str {
        Self::SCRIPT_SOURCE
    }
    pub fn stone_prover_script_source() -> &'static str {
        Self::STONE_PROVER_SCRIPT_SOURCE
    }

    pub async fn cairo_verify(&mut self, cairo_proof_mode: bool) -> Result<(), ClaimProverError> {
        match &self.dir {
            Some(dir) => run_cairo_verify_script(Self::script_source(), dir, cairo_proof_mode)
                .await
                .map_err(ClaimProverError::Cairo)
                .and_then(|_| {
                    self.cairo_output = Some(self.read_output()?);
                    Ok(())
                }),
            None => Err(ClaimProverError::InputFileNameNotSet),
        }
    }

    pub fn cairo_output(&self) -> Option<&CairoVerifierOutput> {
        self.cairo_output.as_ref()
    }

    pub async fn stone_prove(&self, force_stone_proving: bool) -> Result<String, ClaimProverError> {
        match &self.dir {
            Some(dir) => {
                run_stone_prover_script(Self::STONE_PROVER_SCRIPT_SOURCE, dir, force_stone_proving)
                    .await
            }
            None => Err(ClaimProverError::InputFileNameNotSet),
        }
    }
    pub fn stone_proof(&self) -> anyhow::Result<StoneProof> {
        self.stone_proof_file
            .as_ref()
            .ok_or(anyhow::anyhow!("stone proof file name not set"))
            .and_then(|stone_proof_file| StoneProofJson::try_from_file(stone_proof_file))
            .map(StoneProof::from)
    }

    pub fn file_name(&self) -> Option<&str> {
        self.cairo_input_file.as_deref()
    }
    pub fn cairo_output_file(&self) -> Option<&str> {
        self.cairo_output_file.as_deref()
    }
    pub fn stone_proof_file(&self) -> Option<&str> {
        self.stone_proof_file.as_deref()
    }
}

impl<'a> ClaimProver<'a> {
    fn new(
        merkle_proof: StarknetPedersenMerkleProof,
        rlp: Vec<u8>,
        claim: ClaimSerializable,
        claim_digest_roots: ClaimDigestRoots,
        attestation_chain: FragmentSliceSerializable<'a>,
    ) -> Self {
        Self {
            merkle_proof: (merkle_proof, rlp).into(),
            claim,
            claim_digest_roots,
            attestation_chain,
            cairo_input_file: None,
            cairo_output_file: None,
            stone_proof_file: None,
            dir: None,
            cairo_output: None,
        }
    }

    fn with_default_files(mut self) -> anyhow::Result<Self> {
        let dir = self.default_dir();
        create_dir_all(&dir)?;

        let cairo_input_file = Self::default_cairo_input_file_name(&dir);
        let cairo_output_file = Self::default_cairo_output_file_name(&dir);
        let stone_proof_file = Self::default_stone_proof_file_name(&dir);

        self.to_file(&cairo_input_file)?;

        self.dir = Some(dir);
        self.cairo_input_file = Some(cairo_input_file);
        self.cairo_output_file = Some(cairo_output_file);
        self.stone_proof_file = Some(stone_proof_file);
        Ok(self)
    }

    fn to_file(&self, fname: &str) -> anyhow::Result<()> {
        let file = File::create(fname)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        Ok(writer.flush()?)
    }

    // TODO: make it private, it's public only for dev
    pub fn default_dir(&self) -> String {
        let hex_block_number = format!("0x{:X}", self.claim.id().block_item_id.block_number());
        let subject_index = self.claim.id().block_item_id.index() as usize;

        let partial_dir = &format!(
            "block_{hex_block_number}/{}{subject_index}",
            self.claim.id().kind.subdir()
        );
        format!("{}/{partial_dir}", claim_proof_dir())
    }

    fn default_cairo_input_file_name(dir: &str) -> String {
        format!("{dir}/program_input.json")
    }
    fn default_cairo_output_file_name(dir: &str) -> String {
        format!("{dir}/output.txt")
    }
    fn default_stone_proof_file_name(dir: &str) -> String {
        format!("{dir}/proof.json")
    }

    fn read_output(&self) -> Result<CairoVerifierOutput, ClaimProverError> {
        self.cairo_output_file
            .as_ref()
            .ok_or(ClaimProverError::OutputFileNameNotSet)
            .and_then(|cairo_output_file| {
                let output_str = std::fs::read_to_string(cairo_output_file)
                    .map_err(|err| ClaimProverError::OutputParseFailure(format!("{err:?}")))?;

                CairoVerifierOutput::try_from_prefixed_str(&output_str[..])
                    .map_err(|err| ClaimProverError::OutputParseFailure(format!("{err:?}")))
            })
    }
}

pub async fn build_prover<'a>(
    claim: ClaimSerializable,
    attestation_chain_slice: FragmentSlice<'a>,
    tx_bytes: Vec<Vec<u8>>,
    rx_bytes: Vec<Vec<u8>>,
) -> Result<ClaimProver<'a>, ClaimProverError> {
    let (transactions_tree, receipts_tree) =
        futures::future::join(async { StarknetPedersenMmr::from(&tx_bytes[..]) }, async {
            StarknetPedersenMmr::from(&rx_bytes[..])
        })
        .await;

    let subject_index = claim.id().block_item_id.index() as usize;
    let (subject_bytes, merkle_path) = match claim.id().kind {
        ClaimKind::Tx => (
            tx_bytes
                .get(subject_index)
                .ok_or(ClaimProverError::Other(format!(
                    "tx index {subject_index} is out of bounds"
                )))?
                .clone(),
            transactions_tree.generate_proof(subject_index),
        ),
        ClaimKind::Rx => (
            rx_bytes
                .get(subject_index)
                .ok_or(ClaimProverError::Other(format!(
                    "rx index {subject_index} is out of bounds"
                )))?
                .clone(),
            receipts_tree.generate_proof(subject_index),
        ),
    };
    let digest_roots = ClaimDigestRoots::new(&transactions_tree.root().0, &receipts_tree.root().0);

    let instance = ClaimProver::new(
        merkle_path,
        subject_bytes,
        claim,
        digest_roots,
        attestation_chain_slice.into(),
    )
    .with_default_files()
    .map_err(|err| ClaimProverError::SerializationFailure(format!("{err:?}")))?;

    Ok(instance)
}

async fn run_cairo_verify_script(
    script_source: &str,
    input_dir: &str,
    cairo_proof_mode: bool,
) -> Result<(), ScriptError> {
    tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!(
            "source {} {} {}",
            script_source,
            input_dir,
            if cairo_proof_mode { "proof_mode" } else { "" },
        ))
        .stdout(std::process::Stdio::inherit())
        .output()
        .await
        .map_err(|_err| ScriptError::ProcessExecutionFailure)
        .and_then(|output| {
            output.status.success().then_some(()).ok_or({
                let _ = std::io::stdout().write_all(&output.stdout);
                let _ = std::io::stdout().write_all(&output.stderr);

                output.status.code().into()
            })
        })
}

async fn run_stone_prover_script(
    script_source: &str,
    input_dir: &str,
    force_stone_proving: bool,
) -> Result<String, ClaimProverError> {
    let output = tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!(
            "source {} {} {}",
            script_source,
            input_dir,
            if force_stone_proving { "force" } else { "" }
        ))
        .stdout(std::process::Stdio::inherit())
        .output()
        .await
        .map_err(|_err| ScriptError::ProcessExecutionFailure)?;
    if output.status.code() == Some(43) {
        return Ok("WARNING: proof file already exists, skipping stone-proving. Use force-stone-proving flag for forcing stone-proving".to_owned());
    }
    if output.status.success() {
        Ok("done".to_owned())
    } else {
        let _ = std::io::stdout().write_all(&output.stdout);
        let _ = std::io::stdout().write_all(&output.stderr);

        Err(ClaimProverError::Cairo(output.status.code().into()))
    }
}
