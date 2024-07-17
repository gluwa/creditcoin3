use anyhow::anyhow;
use colored::Colorize;
use either::Either;
use tracing::debug;

use prover_primitives::claim::ClaimSerializable;
use prover_primitives::types::{CairoVerifierOutput, ClaimProverError, StoneProof};

use attestation_chain::attestation_checkpoints::{AttestationCheckpoint, AttestationCheckpoints};
use attestation_chain::attestation_fragment::AttestationFragment;

use crate::claim_prover::{build_prover, ClaimProver};

pub mod claim_prover;

pub mod json_serializable;

pub async fn cairo_generate_proof(
    claim: ClaimSerializable,
    claim_attestation_fragment: &AttestationFragment,
    tx_bytes: Vec<Vec<u8>>,
    rx_bytes: Vec<Vec<u8>>,
    cairo_proof_mode: bool,
    force_stone_proving: bool,
) -> anyhow::Result<either::Either<(StoneProof, String), CairoVerifierOutput>> {
    let claim_block_number = claim.id().block_item_id.block_number();
    let attestation_chain_slice = claim_attestation_fragment.attestation_slice_for(claim_block_number, None)
        .ok_or(anyhow!("can't create attestation checkpoint slice for {} on this attestation chain ({:?}, {:?})",
            claim_block_number,
            claim_attestation_fragment.tail().map(|att| att.n()),
            claim_attestation_fragment.head().map(|att| att.n())))?;

    debug!("\n");
    debug!("---------- cairo claim proving task is starting ----------");
    debug!("claim: {:?}", claim);
    debug!("fetching block and building merkle trees...");

    let mut cairo_verifier = build_prover(claim.clone(), attestation_chain_slice,tx_bytes, rx_bytes)
        .await
        .map(|claim_cairo_verifier| {
            debug!("done");
            debug!("\ncairo0 input file {}", format!("{:?}", claim_cairo_verifier.file_name()).bright_cyan());
            debug!("running script {}", format!("{:?}", ClaimProver::script_source()).bright_cyan());
            claim_cairo_verifier
        })
        .map_err(|err| {
            anyhow!("{}",
                match &err {
                    ClaimProverError::AttestationFragmentMismatch(b, tail, head) =>
                        format!("can't create attestation checkpoint slice for {b} on this attestation chain ({tail:?}, {head:?})"),
                    ClaimProverError::BlockFetchFailure(msg) =>
                        format!("failure while fetching block corresponding claim: {msg}"),
                    ClaimProverError::ClaimNotIdentified =>
                        format!("claim {claim:?} was not identified in the block"),
                    err => format!("could not build verifier: {err:?}"),
                }
            )
        })?;

    cairo_verifier
        .cairo_verify(cairo_proof_mode)
        .await
        .map_err(|err| anyhow!("{err:?}"))?;

    let output = cairo_verifier
        .cairo_output()
        .ok_or(anyhow!("successful verification expected to yield output"))?;
    debug!("----- cairo verification successful -----");
    debug!("cairo verification output:");
    debug!("{}", format!("{:?}", output).bold());

    let mut checkpoints = AttestationCheckpoints::new();
    checkpoints
        .try_append(
            claim_attestation_fragment
                .checkpoint()
                .expect("attestation fragment expected to be full"),
        )
        .map_err(|err| anyhow!("{:?}", err))?;

    let output_checkpoint = AttestationCheckpoint::try_from_block(
        output.continuity_checkpoint_block_number,
        output.continuity_checkpoint_digest,
    )
    .ok_or(anyhow!(
        "expected to get a valid checkpoint from cairo verifier's output"
    ))?;

    if checkpoints.verify_claim_continuity(&output_checkpoint) {
        debug!(
            "{}",
            format!(
                "\nclaim continuity validated at checkpoint: {:?}",
                output_checkpoint
            )
            .green()
        );
    } else {
        return Err(anyhow!(
            "claim continuity not validated on attestation chain"
        ));
    };

    if cairo_proof_mode {
        debug!("running stone-prover, will take a while...");

        cairo_verifier
            .stone_prove(force_stone_proving)
            .await
            .map(|msg| {
                println!("{msg}");
            })
            .map_err(|err| anyhow!("{err:?}"))?;

        cairo_verifier
            .stone_proof()
            .map(|stone_proof| Either::Left((stone_proof, cairo_verifier.default_dir())))
    } else {
        Ok(Either::Right(output.clone()))
    }
}

#[allow(dead_code)]
async fn run_stone_verify_script(script_source: &str, input_dir: &str) -> anyhow::Result<()> {
    use std::io::Write;

    tokio::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!("source {} {}", script_source, input_dir,))
        .stdout(std::process::Stdio::inherit())
        .output()
        .await
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .and_then(|output| {
            output.status.success().then_some(()).ok_or({
                let _ = std::io::stdout().write_all(&output.stdout);
                let _ = std::io::stdout().write_all(&output.stderr);

                anyhow::anyhow!("error code: {:?}", output.status.code())
            })
        })
}

#[cfg(test)]
mod tests {
    use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
    use attestation_db::AttestationDB;
    use colored::Colorize;
    use eth_common::{transaction::BlockItem, Client};
    use hashbrown::HashSet;
    use prover_primitives::types::StoneProofPublicInput;
    use prover_primitives::{
        claim::{Claim, ClaimIdentifier, ClaimKind, ClaimSerializable},
        claim_query::{Eip4844TxClaimQueryField::*, TxClaimQuery},
    };
    use utils::{block_item_traits::BlockItemIdentifier, utils::felts_from_bytes};

    /// tests this circuit:
    /// claim submission to prover -> running cairo program on prover (and proof gen) -> proof verification on claimer
    /// prior to running this test:
    /// - config.json with API provider urls must be present in the project's workspace root (see config_template.json)
    /// - run 'cargo run -- --from-block 19543670' in 'attestor-online-sim' directory to generate a short range of checkpoints
    /// - run 'cargo run' (with --reset-db flag for the first time) in 'prover-attestation-db-online-builder' directory
    /// to create attestation db on prover's side
    #[tokio::test]
    async fn claim_validation_test() {
        const SCRIPT_SOURCE: &str = "../cairo/scripts/verify_merkle_proof.sh";

        let block = 19543673;
        let index = 95;
        // access token should not be published on github
        let url = "wss://eth-mainnet.g.alchemy.com/v2/ziEK05XpthEPz4a3g1iA4iD828g6wm_e";
        let checkpoints_path = "../data/execution-chain";

        let eth_client = Client::new(url).await.expect("failed to create eth client");
        // -------------------------------------- claimer part ----------------------------------
        let tx_asd = eth_client.get_transactions(block).await.unwrap();
        let rx_asd = eth_client.get_receipts(block).await.unwrap();

        let tx_bytes = tx_asd
            .iter()
            .map(eth_common::transaction::Transaction::to_bytes)
            .collect::<Vec<_>>();
        let rx_bytes = rx_asd
            .iter()
            .map(eth_common::transaction::Receipt::to_bytes)
            .collect::<Vec<_>>();

        // rlp-encoded tx/rx
        let payload_bytes = tx_asd[index].payload_bytes();
        // create rlp instance containing payload bytes
        let rlp = rlp::Rlp::new(&payload_bytes[..]);
        // form claim id
        let claim_id = ClaimIdentifier {
            kind: ClaimKind::Tx,
            block_item_id: BlockItemIdentifier::new(block.into(), index as u64),
        };
        // form query of fields of interest to get values from prover for
        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                SingleDataRelativeRange(Some(24..30)),
                Nonce,
                SingleDataRelativeRange(Some(33..39)),
                BlobVersionedHashes(Some(0)),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, rlp).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        // ----------------------- prover's part ------------------------------------------------
        // internal prover's data
        let db_url = "../data/db";
        let db = attestation_db::json_db::AttestationJsonDB::try_create(db_url).unwrap();
        let attestation_fragment = db.get_fragment_for(block.into()).unwrap().into();

        let mut checkpoints =
            AttestationCheckpointsForDev::with_execution_chain_url(checkpoints_path);
        // simulate polling checkpoints from CC3 blockchain
        checkpoints.poll().unwrap();

        // change to false if you don't want to generate stone-proof and rather use output of cairo program
        let generate_stone_proof = true;
        let overwrite_existing_stone_proof = false;
        let result = crate::cairo_generate_proof(
            cairo_claim,
            &attestation_fragment,
            tx_bytes,
            rx_bytes,
            generate_stone_proof,
            overwrite_existing_stone_proof,
        )
        .await;

        // -------------------------------------- claimer part ----------------------------------
        let cairo_output_or_stone_proof = result.unwrap();

        let output = match cairo_output_or_stone_proof {
            either::Left((mut stone_proof, stone_proof_dir)) => {
                crate::run_stone_verify_script(SCRIPT_SOURCE, &stone_proof_dir)
                    .await
                    .unwrap();
                println!("{}", "CLAIMER: proof stone-verified".bold().green());

                stone_proof
                    .strip_off_annotations()
                    .strip_off_prover_config()
                    .strip_off_private_input();
                StoneProofPublicInput::try_from(stone_proof.proof()).unwrap()
            }
            either::Right(cairo_output) => cairo_output,
        };

        let checkpoint = checkpoints.inner().checkpoint_for(block.into()).unwrap();
        assert_eq!(output.continuity_checkpoint_block_number, checkpoint.n());
        assert_eq!(&output.continuity_checkpoint_digest, checkpoint.digest());
        println!("{}", "CLAIMER: continuity verified".bold().green());

        claim.validate_fields(&output).unwrap();

        println!(
            "{}",
            "CLAIMER: query fields and hash validated".bold().green()
        );
    }

    #[tokio::test]
    async fn tx_output_matches_rlp_test() {
        let block = 19543696;
        let index = 45;

        let url = "wss://eth-mainnet.g.alchemy.com/v2/ziEK05XpthEPz4a3g1iA4iD828g6wm_e";

        let eth_client = Client::new(url).await.expect("failed to create eth client");
        // -------------------------------------- claimer part ----------------------------------
        let tx_asd = eth_client.get_transactions(block).await.unwrap();

        let payload_bytes = tx_asd[index].payload_bytes();

        let rlp = rlp::Rlp::new(&payload_bytes[..]);
        let rlp_felts = felts_from_bytes(rlp.as_raw());

        let claim_id = ClaimIdentifier {
            kind: ClaimKind::Tx,
            block_item_id: BlockItemIdentifier::new(block.into(), index as u64),
        };

        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                //                SingleDataRelativeRange(Some(24..30)),
                Nonce,
                SingleDataRelativeRange(None),
                //                SingleDataRelativeRange(Some(33..39)),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();

        let claim = Claim::try_create(claim_id.clone(), claim_query, rlp).unwrap();

        //        let felts_from_prover = rlp_felts.clone();
        let prover_output = StoneProofPublicInput {
            claim_id,
            continuity_checkpoint_digest: Default::default(),
            continuity_checkpoint_block_number: Default::default(),
            query_hash: claim.query_hash(),
            claim_fields: rlp_felts,
        };

        assert!(claim.validate_fields(&prover_output).is_ok());
    }
}
