use crate::fragment::{FragmentSlice, FragmentSliceSerializable};
use crate::types::{
    CairoVerifierOutput, ClaimDigestRoots, ClaimProverError, MerkleProofWithClaimJson, ScriptError,
};
use attestor::merkle::tree::{StarknetPedersenMerkleProof, StarknetPedersenMmr};
use mmr::traits::MerkleTreeTrait;
use prover_primitives::claim::{Claim, ClaimKind};
use serde::Serialize;
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use tempfile::TempDir;

const DATA_ROOT_DIR: &str = "../data";
const CLAIM_PROOF_DIR: &str = "claim-proofs";

const SCRIPT_SOURCE: &str = "../cairo/scripts/verify_merkle_proof.sh";

const STONE_PROVER_SCRIPT_SOURCE: &str = "../cairo/scripts/stone_prove_claim.sh";

fn claim_proof_dir() -> String {
    format!("{}/{}", DATA_ROOT_DIR, CLAIM_PROOF_DIR)
}

#[derive(Serialize)]
pub struct ClaimProver<'a> {
    claim_with_merkle_proof: MerkleProofWithClaimJson,
    claim_digest_roots: ClaimDigestRoots,
    attestation_chain: FragmentSliceSerializable<'a>,
    claim_block_number: u64,
    claim_kind: ClaimKind,
    claim_index: usize,
    cairo_input_file: Option<String>,
    cairo_output_file: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    dir: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    cairo_output: Option<CairoVerifierOutput>,
}

impl<'a> ClaimProver<'a> {
    fn new(
        merkle_proof: StarknetPedersenMerkleProof,
        rlp: Vec<u8>,
        claim_block_number: u64,
        claim_kind: ClaimKind,
        claim_index: usize,
        claim_digest_roots: ClaimDigestRoots,
        attestation_chain: FragmentSliceSerializable<'a>,
    ) -> Self {
        Self {
            claim_with_merkle_proof: (merkle_proof, rlp, claim_kind).into(),
            claim_digest_roots,
            attestation_chain,
            claim_block_number,
            claim_kind,
            claim_index,
            cairo_output_file: None,
            cairo_input_file: None,
            dir: None,
            cairo_output: None,
        }
    }

    // fn with_temp_files(mut self) -> anyhow::Result<Self> {
    //     // unused for now
    //     let _default_dir =
    //         self.default_dir(self.claim_block_number, self.claim_kind, self.claim_index);
    //
    //     let temp_dir = TempDir::new()?;
    //     let dir = temp_dir
    //         .path()
    //         .to_str()
    //         .ok_or_else(|| {
    //             anyhow::Error::msg("Failed to convert temporary directory path to string")
    //         })?
    //         .to_string();
    //
    //     let cairo_input_file = Self::default_cairo_input_file_name(&dir);
    //     let cairo_output_file = Self::default_cairo_output_file_name(&dir);
    //
    //     self.to_temp_file(&cairo_input_file)?;
    //
    //     self.temp_dir = Some(temp_dir);
    //     self.cairo_input_file = Some(cairo_input_file);
    //     self.cairo_output_file = Some(cairo_output_file);
    //     Ok(self)
    // }

    fn with_default_files(mut self) -> anyhow::Result<Self> {
        let dir = self.default_dir(self.claim_block_number, self.claim_kind, self.claim_index);

        create_dir_all(&dir)?;

        let cairo_input_file = Self::default_cairo_input_file_name(&dir);
        let cairo_output_file = Self::default_cairo_output_file_name(&dir);

        self.to_file(&cairo_input_file)?;

        self.dir = Some(dir);
        self.cairo_input_file = Some(cairo_input_file);
        self.cairo_output_file = Some(cairo_output_file);
        Ok(self)
    }

    fn default_cairo_input_file_name(dir: &str) -> String {
        format!("{dir}/program_input.json")
    }

    fn default_cairo_output_file_name(dir: &str) -> String {
        format!("{dir}/output.txt")
    }

    fn default_dir(&self, block_number: u64, claim_kind: ClaimKind, claim_index: usize) -> String {
        let hex_block_number = format!("0x{:X}", block_number);

        let partial_dir = &format!(
            "block_{hex_block_number}/{}{claim_index}",
            claim_kind.subdir()
        );
        format!("{}/{partial_dir}", claim_proof_dir())
    }

    fn to_file(&self, fname: &str) -> anyhow::Result<()> {
        let file = File::create(fname)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        Ok(writer.flush()?)
    }

    pub fn script_source() -> &'static str {
        SCRIPT_SOURCE
    }

    pub async fn cairo_verify(mut self, proof_mode: bool) -> Result<Self, ClaimProverError> {
        match &self.dir {
            Some(dir) => run_cairo_verify(Self::script_source(), dir, proof_mode)
                .await
                .map_err(ClaimProverError::Cairo)
                .and_then(|_| {
                    self.cairo_output = Some(self.read_output()?);
                    Ok(self)
                }),
            None => Err(ClaimProverError::InputFileNameNotSet),
        }
    }

    pub fn take_output(&mut self) -> Option<CairoVerifierOutput> {
        self.cairo_output.take()
    }

    fn read_output(&self) -> Result<CairoVerifierOutput, ClaimProverError> {
        self.cairo_output_file
            .as_ref()
            .ok_or(ClaimProverError::OutputFileNameNotSet)
            .and_then(|cairo_output_file| {
                let output_str = std::fs::read_to_string(cairo_output_file)
                    .map_err(|err| ClaimProverError::OutputParseFailure(format!("{err:?}")))?;

                CairoVerifierOutput::try_from(&output_str[..])
                    .map_err(|err| ClaimProverError::OutputParseFailure(format!("{err:?}")))
            })
    }

    pub fn file_name(&self) -> Option<&str> {
        self.cairo_input_file.as_deref()
    }

    pub async fn stone_prove(self, stone_proof_mode: bool) -> Result<String, ClaimProverError> {
        match &self.dir {
            Some(dir) => run_stone_prover(STONE_PROVER_SCRIPT_SOURCE, dir, stone_proof_mode).await,
            None => Err(ClaimProverError::InputFileNameNotSet),
        }
    }

    pub async fn build_prover<Address>(
        claim: Claim<Address>,
        attestation_chain_slice: FragmentSlice<'_>,
        // every tx in a given block in order
        tx_bytes: Vec<Vec<u8>>,
        // every rx in a given block in order
        rx_bytes: Vec<Vec<u8>>,
    ) -> Result<ClaimProver<'_>, ClaimProverError> {
        let claim_block_number: u64 = claim.block_number;

        // this is good for now
        let claim_index = claim.tx_index;

        let (transaction_tree, receipt_tree) =
            futures::future::join(async { StarknetPedersenMmr::from(&tx_bytes[..]) }, async {
                StarknetPedersenMmr::from(&rx_bytes[..])
            })
            .await;

        let (claim_bytes, merkle_path) = match claim.kind {
            ClaimKind::Tx => (
                tx_bytes[claim_index as usize].clone(),
                transaction_tree.generate_proof(claim_index as usize),
            ),
            ClaimKind::Rx => (
                rx_bytes[claim_index as usize].clone(),
                receipt_tree.generate_proof(claim_index as usize),
            ),
        };

        let digest_roots =
            ClaimDigestRoots::new(&transaction_tree.root().0, &receipt_tree.root().0);

        let prover = ClaimProver::new(
            merkle_path,
            claim_bytes,
            claim_block_number,
            claim.kind,
            claim_index as usize,
            digest_roots,
            attestation_chain_slice.into(),
        )
        .with_default_files()
        .map_err(|err| ClaimProverError::SerializationFailure(format!("{err:?}")))?;

        Ok(prover)
    }
}

async fn run_cairo_verify(script: &str, dir: &str, proof_mode: bool) -> Result<(), ScriptError> {
    tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!(
            "source {} {} {}",
            script,
            dir,
            if proof_mode { "proof_mode" } else { "" },
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

async fn run_stone_prover(
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
        return Ok("WARNING: proof file already exists, skipping stone-proving. Use force_stone_proving flag for forcing stone-proving".to_owned());
    }
    if output.status.success() {
        Ok("done".to_owned())
    } else {
        let _ = std::io::stdout().write_all(&output.stdout);
        let _ = std::io::stdout().write_all(&output.stderr);

        Err(ClaimProverError::Cairo(output.status.code().into()))
    }
}
