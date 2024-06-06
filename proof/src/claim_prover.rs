use crate::eth;
use crate::fragment::FragmentSlice;
use crate::types::{
    CairoVerifierOutput, ClaimDigestRoots, ClaimProverError, MerkleProofWithClaimJson, ScriptError,
};
use attestor::merkle::tree::{StarknetPedersenMerkleProof, StarknetPedersenMmr};
use attestor::transaction::BlockItem;
use eth::{fetch_block_receipts, fetch_block_transactions};
use mmr::traits::MerkleTreeTrait;
use prover_primitives::claim::{Claim, ClaimKind};

pub struct ClaimProver<'a, H: Clone, A> {
    claim_with_merkle_proof: MerkleProofWithClaimJson,
    claim_digest_roots: ClaimDigestRoots,
    attestation_chain: FragmentSlice<'a, H, A>,
    claim_block_number: u64,
    claim_kind: ClaimKind,
    claim_index: usize,
    fname: Option<String>,
    cairo_output_file: Option<String>,
    dir: Option<String>,
    cairo_output: Option<CairoVerifierOutput>,
}

impl<'a, H, A> ClaimProver<'a, H, A>
where
    H: Clone,
{
    fn new(
        merkle_proof: StarknetPedersenMerkleProof,
        rlp: Vec<u8>,
        claim_block_number: u64,
        claim_kind: ClaimKind,
        claim_index: usize,
        claim_digest_roots: ClaimDigestRoots,
        attestation_chain: FragmentSlice<'a, H, A>,
    ) -> Self {
        Self {
            claim_with_merkle_proof: (merkle_proof, rlp, claim_kind).into(),
            claim_digest_roots,
            attestation_chain,
            claim_block_number,
            claim_kind,
            claim_index,
            cairo_output_file: None,
            fname: None,
            dir: None,
            cairo_output: None,
        }
    }

    const SCRIPT_SOURCE: &'static str = "../cairo-scripts/verify_merkle_proof.sh";

    const STONE_PROVER_SCRIPT_SOURCE: &'static str = "../cairo-scripts/stone_prove_claim.sh";

    pub fn script_source() -> &'static str {
        Self::SCRIPT_SOURCE
    }

    pub async fn cairo_verify(mut self, proof_mode: bool) -> Result<Self, ClaimProverError> {
        match &self.dir {
            Some(dir) => run_cairo_verify(Self::script_source(), dir, proof_mode)
                .await
                .map_err(|err| ClaimProverError::Cairo(err))
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
        self.fname.as_ref().map(String::as_str)
    }

    pub async fn stone_prove(self, stone_proof_mode: bool) -> Result<String, ClaimProverError> {
        match &self.dir {
            Some(dir) => {
                run_stone_prover(Self::STONE_PROVER_SCRIPT_SOURCE, dir, stone_proof_mode).await
            }
            None => Err(ClaimProverError::InputFileNameNotSet),
        }
    }
}

async fn run_cairo_verify(script: &str, dir: &str, proof_mode: bool) -> Result<(), ScriptError> {
    use std::io::Write;

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
    use std::io::Write;
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

pub async fn build_prover<'a, H: Clone, A, Address>(
    url: &str,
    claim: Claim<Address>,
    attestation_chain_slice: FragmentSlice<'a, H, A>,
    // every tx in a given block in order
    tx_bytes: Vec<Vec<u8>>,
    // every rx in a given block in order
    rx_bytes: Vec<Vec<u8>>,
) -> Result<ClaimProver<'a, H, A>, ClaimProverError> {
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

    let digest_roots = ClaimDigestRoots::new(&transaction_tree.root().0, &receipt_tree.root().0);

    let prover = ClaimProver::new(
        merkle_path,
        claim_bytes,
        claim_block_number,
        claim.kind,
        claim_index as usize,
        digest_roots,
        attestation_chain_slice.into(),
    );

    Ok(prover)
}
