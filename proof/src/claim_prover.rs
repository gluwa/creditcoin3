use crate::eth;
use crate::fragment::FragmentSlice;
use crate::types::{CairoVerifierOutput, ClaimDigestRoots, MerkleProofWithClaimJson};
use attestor::merkle::tree::{StarknetPedersenMerkleProof, StarknetPedersenMmr};
use attestor::transaction::{BlockItem, Receipt, Transaction};
use eth::{fetch_block_receipts, fetch_block_transactions};
use mmr::traits::MerkleTreeTrait;
use prover_primitives::claim::{Claim, ClaimKind};
use serde::{Deserialize, Serialize};

pub struct ClaimProver<'a, H: Clone, A> {
    claim_with_merkle_proof: MerkleProofWithClaimJson,
    claim_digest_roots: ClaimDigestRoots,
    attestation_chain: FragmentSlice<'a, H, A>,
    claim_block_number: u64,
    claim_kind: ClaimKind,
    claim_index: usize,
    cairo_output_file: Option<String>,
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
            cairo_output: None,
        }
    }
}

pub struct ClaimProverError {}

pub async fn build_prover<'a, H: Clone, A, Address>(
    url: &str,
    claim: Claim<Address>,
    attestation_chain_slice: FragmentSlice<'a, H, A>,
) -> Result<ClaimProver<'a, H, A>, ClaimProverError> {
    let claim_block_number: u64 = claim.block_number;

    let block_tx = fetch_block_transactions(url, claim_block_number)
        .await
        .unwrap();
    let block_rx = fetch_block_receipts(url, claim_block_number).await.unwrap();

    // ToDo: do the actual check and find the claim index in tx and rx
    // let claim_index = match claim.kind {
    //     ClaimKind::Tx => block_tx.find_claim(&claim)?.index(),
    //     ClaimKind::Rx => block_rx.find_claim(&claim)?.index(),
    // };

    // this is good for now
    let claim_index = claim.tx_index;

    let tx_bytes = block_tx
        .into_iter()
        .map(|tx| tx.to_bytes())
        .collect::<Vec<Vec<u8>>>();
    let rx_bytes = block_rx
        .into_iter()
        .map(|rx| rx.to_bytes())
        .collect::<Vec<Vec<u8>>>();

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
