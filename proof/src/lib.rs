use anyhow::anyhow;
use colored::Colorize;
use either::Either;
use eth_common::OrderedBlock;
use tracing::{debug, info};

use attestation_chain::attestation_fragment::AttestationFragment;
use prover_primitives::claim::ClaimSerializable;
use prover_primitives::types::{CairoVerifierOutput, ClaimProverError, StoneProof};

use crate::claim_prover::{build_prover, ClaimProver};

pub mod claim_prover;
pub mod json_serializable;

pub async fn cairo_generate_proof(
    claim: ClaimSerializable,
    claim_attestation_fragment: &AttestationFragment,
    block: OrderedBlock,
    cairo_proof_mode: bool,
    force_stone_proving: bool,
) -> anyhow::Result<either::Either<(StoneProof, String), CairoVerifierOutput>> {
    let claim_block_number = claim.id().block_number();
    let checkpoint_block_number = claim_attestation_fragment
        .head()
        .ok_or(anyhow!("no fragment head"))?
        .block_number;

    let claim_blocks_slice = claim_attestation_fragment
        .blocks_slice_for(claim_block_number)
        .ok_or(anyhow!("unable to slice fragment {claim_attestation_fragment:?} for block number {} and checkpoint {}", claim_block_number, checkpoint_block_number))?;

    debug!("\n");
    info!("---------- cairo claim proving task is starting ----------");
    debug!("claim: {:?}", claim);
    debug!("fetching block and building merkle trees...");

    let mut cairo_verifier = build_prover(claim.clone(), claim_blocks_slice, block)
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

    info!("----- cairo verification successful -----");
    debug!("cairo verification output:");
    debug!("{}", format!("{:?}", output).bold());

    if cairo_proof_mode {
        info!("running stone-prover, will take a while...");

        cairo_verifier
            .stone_prove(force_stone_proving)
            .await
            .map(|msg| {
                info!("{}", msg);
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
pub async fn run_stone_verify_script(script_source: &str, input_dir: &str) -> anyhow::Result<()> {
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
    use super::ClaimProver;
    use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
    use attestation_chain::AttestationChainParams;
    use attestation_chain::ETH_ATTESTATION_CHAIN_PARAMS_DEV;
    use block_cache::{BlockCache, CacheT};
    use colored::Colorize;
    use eth_common::OrderedBlock;
    use hashbrown::HashSet;
    use prover_primitives::stark_program_auth::{
        StarkProgramAuth, StarkProgramAuthError, StarkProgramMetadataStorage,
    };
    use prover_primitives::types::CairoVerifierOutput;
    use prover_primitives::types::StoneProofPublicInput;
    use prover_primitives::{
        claim::{Claim, ClaimIdentifier, ClaimSerializable},
        claim_query::TxClaimQuery,
    };
    use utils::block_item_traits::BlockItem;

    /// tests this circuit:
    /// claim submission to prover -> running cairo program on prover (and proof gen) -> proof verification on claimer
    /// prior to running this test:
    /// - config.json with API provider urls must be present in the project's workspace root (see config_template.json)
    /// - run 'cargo run -- --from-block 19543670' in 'attestor-online-sim' directory to generate a short range of checkpoints
    /// - run 'cargo run' (with --reset-db flag for the first time) in 'prover-attestation-db-online-builder' directory
    /// to create attestation db on prover's side
    #[ignore]
    #[test]
    fn claim_validation_test_tx_type_0() {
        use prover_primitives::claim_query::LegacyClaimQueryField::*;

        let block_number = 19543673u64;
        let index = 13;

        // let poc_config = PocConfig::try_from_file("../config.json").unwrap();

        // let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // // -------------------------------------- claimer part ----------------------------------
        // let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        //        println!("{:?}", block.items()[index]);
        let payload_bytes = block.items()[index].payload_bytes();
        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                SingleDataRelativeRange(Some(24..30)),
                Nonce,
                SingleDataRelativeRange(Some(33..39)),
                Signature,
                SignatureHash,
                StateRoot,
                UsedGas,
                LogsBloom,
                SingleLog(Some(0)),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert!(validate_proof_data(
            ETH_ATTESTATION_CHAIN_PARAMS_DEV,
            block_number,
            claim,
            output
        )
        .is_ok());

        println!(
            "{}",
            "CLAIMER: query fields and hash validated".bold().green()
        );
    }

    /// tests this circuit:
    /// claim submission to prover -> running cairo program on prover (and proof gen) -> proof verification on claimer
    /// prior to running this test:
    /// - config.json with API provider urls must be present in the project's workspace root (see config_template.json)
    /// - run 'cargo run -- --from-block 19543670' in 'attestor-online-sim' directory to generate a short range of checkpoints
    /// - run 'cargo run' (with --reset-db flag for the first time) in 'prover-attestation-db-online-builder' directory
    /// to create attestation db on prover's side
    #[ignore]
    #[test]
    fn claim_validation_test_tx_type_1() {
        use prover_primitives::claim_query::Eip2930ClaimQueryField::*;

        let block_number = 19543676u64;
        let index = 116;

        // let poc_config = PocConfig::try_from_file("../config.json").unwrap();

        // let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // // -------------------------------------- claimer part ----------------------------------
        // let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        //        println!("{:?}", block.items()[index]);
        let payload_bytes = block.items()[index].payload_bytes();
        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                SingleDataRelativeRange(None),
                Nonce,
                Signature,
                SignatureHash,
                StatusCode,
                UsedGas,
                LogsBloom,
                SingleLog(None),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert!(validate_proof_data(
            ETH_ATTESTATION_CHAIN_PARAMS_DEV,
            block_number,
            claim,
            output
        )
        .is_ok());

        println!(
            "{}",
            "CLAIMER: query fields and hash validated".bold().green()
        );
    }

    /// tests this circuit:
    /// claim submission to prover -> running cairo program on prover (and proof gen) -> proof verification on claimer
    /// prior to running this test:
    /// - config.json with API provider urls must be present in the project's workspace root (see config_template.json)
    /// - run 'cargo run -- --from-block 19543670' in 'attestor-online-sim' directory to generate a short range of checkpoints
    /// - run 'cargo run' (with --reset-db flag for the first time) in 'prover-attestation-db-online-builder' directory
    /// to create attestation db on prover's side
    #[ignore]
    #[test]
    fn claim_validation_test_tx_type_2() {
        use prover_primitives::claim_query::Eip1559ClaimQueryField::*;

        let block_number = 19543673u64;
        let index = 96;

        //        let poc_config = PocConfig::try_from_file("../config.json").unwrap();

        //        let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        //        println!("{:?}", block.items()[index]);
        let payload_bytes = block.items()[index].payload_bytes();
        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                SingleDataRelativeRange(Some(24..30)),
                Nonce,
                SingleDataRelativeRange(Some(33..39)),
                Signature,
                SignatureHash,
                StatusCode,
                UsedGas,
                LogsBloom,
                SingleLog(Some(4)),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert!(validate_proof_data(
            ETH_ATTESTATION_CHAIN_PARAMS_DEV,
            block_number,
            claim,
            output
        )
        .is_ok());

        println!(
            "{}",
            "CLAIMER: query fields and hash validated".bold().green()
        );
    }

    /// tests this circuit:
    /// claim submission to prover -> running cairo program on prover (and proof gen) -> proof verification on claimer
    /// prior to running this test:
    /// - config.json with API provider urls must be present in the project's workspace root (see config_template.json)
    /// - run 'cargo run -- --from-block 19543670' in 'attestor-online-sim' directory to generate a short range of checkpoints
    /// - run 'cargo run' (with --reset-db flag for the first time) in 'prover-attestation-db-online-builder' directory
    /// to create attestation db on prover's side
    #[ignore]
    #[test]
    fn claim_validation_test_tx_type_3() {
        use prover_primitives::claim_query::Eip4844ClaimQueryField::*;

        let block_number = 19543673u64;
        let index = 95;

        //        let poc_config = PocConfig::try_from_file("../config.json").unwrap();

        //        let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // -------------------------------------- claimer part ----------------------------------
        //        let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        let payload_bytes = block.items()[index].payload_bytes();

        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query = TxClaimQuery::try_from(
            vec![
                To,
                SingleDataRelativeRange(Some(24..30)),
                Nonce,
                SingleDataRelativeRange(Some(33..39)),
                SingleDataRelativeRange(None),
                BlobVersionedHashes(Some(0)),
                Signature,
                StatusCode,
                UsedGas,
                LogsBloom,
                SingleLog(None),
            ]
            .into_iter()
            .collect::<HashSet<_>>(),
        )
        .unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert!(validate_proof_data(
            ETH_ATTESTATION_CHAIN_PARAMS_DEV,
            block_number,
            claim,
            output
        )
        .is_ok());

        println!(
            "{}",
            "CLAIMER: query fields and hash validated".bold().green()
        );
    }

    #[ignore]
    #[test]
    fn claim_out_of_bound_test() {
        use prover_primitives::claim_query::Eip4844ClaimQueryField::*;

        let block_number = 19543673u64;
        let index = 95;
        let out_of_bound_index = 1000 + index;
        //        let poc_config = PocConfig::try_from_file("../config.json").unwrap();

        // let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // -------------------------------------- claimer part ----------------------------------
        //        let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        let num_of_leaves = block.items().len();
        let payload_bytes = block.items()[index].payload_bytes();

        // create rlp instance containing payload bytes
        //        let rlp = rlp::Rlp::new(&payload_bytes[..]);
        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, out_of_bound_index as u64);

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
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert_eq!(
            Err(
                prover_primitives::claim::ClaimValidationError::ClaimOutOfBounds(
                    num_of_leaves as u64
                )
            ),
            validate_proof_data(
                ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                block_number,
                claim,
                output
            )
        );

        println!(
            "{}",
            format!("CLAIMER: claim out of bounds, witness at {}", num_of_leaves)
                .bold()
                .red()
        );
    }

    #[ignore]
    #[test]
    fn claim_out_of_bound_null_leaf_test() {
        use prover_primitives::claim_query::Eip4844ClaimQueryField::*;

        let block_number = 19543696u64;
        let index = 156;
        let out_of_bound_index = 1 + index;
        // let poc_config = PocConfig::try_from_file("../config.json").unwrap();
        // let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        //let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // // -------------------------------------- claimer part ----------------------------------
        // let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();

        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        let num_of_leaves = block.items().len();
        let payload_bytes = block.items()[index].payload_bytes();

        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, out_of_bound_index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query =
            TxClaimQuery::try_from(vec![To].into_iter().collect::<HashSet<_>>()).unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));

        assert_eq!(
            Err(
                prover_primitives::claim::ClaimValidationError::ClaimOutOfBounds(
                    num_of_leaves as u64
                )
            ),
            validate_proof_data(
                ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                block_number,
                claim,
                output
            )
        );

        println!(
            "{}",
            format!("CLAIMER: claim out of bounds, witness at {}", num_of_leaves)
                .bold()
                .red()
        );
    }

    // #[tokio::test]
    // async fn claim_first_leaf_out_of_bound_test() {
    //     let block = 19543673u64;
    //     let index = 127;
    //     // rlp-encoded tx/rx
    //     let (payload_bytes, num_of_leaves ) = prepare_claim_subject_rlp(block, index).await;
    //     // create rlp instance containing payload bytes
    //     let rlp = rlp::Rlp::new(&payload_bytes[..]);

    //     let out_of_bound_index = 1 + index;
    //     // form claim id
    //     let claim_id = ClaimIdentifier {
    //         block_item_id: BlockItemIdentifier::new(block.into(), out_of_bound_index as u64),
    //     };
    //     // form query of fields of interest to get values from prover for
    //     let claim_query = TxClaimQuery::try_from(
    //         vec![
    //             To,
    //         ]
    //         .into_iter()
    //         .collect::<HashSet<_>>(),
    //     )
    //     .unwrap();
    //     // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
    //     let claim = Claim::try_create(claim_id, claim_query, rlp).unwrap();
    //     // cairo_claim is sent by claimer to prover
    //     let cairo_claim = ClaimSerializable::from(&claim);

    //     println!("{}", format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold());
    //     // ----------------------- prover's part ------------------------------------------------
    //     let output = run_prover(block, cairo_claim).await;

    //     assert_eq!(
    //         Err(prover_primitives::claim::ClaimValidationError::ClaimOutOfBounds(num_of_leaves as u64)),
    //         validate_proof_data(block, claim, output)
    //     );

    //     println!(
    //         "{}",
    //         format!("CLAIMER: claim out of bounds, witness at {}", num_of_leaves).bold().red()
    //     );
    // }

    #[ignore]
    #[test]
    fn claim_out_of_bound_empty_block_test() {
        use prover_primitives::claim_query::Eip4844ClaimQueryField::*;

        // THIS BLOCK IS EMPTY ON ETHEREUM MAINNET
        let block_number = 19543675u64;
        let fake_block_just_for_rlp = 19543673u64;
        let index = 95;
        //        let poc_config = PocConfig::try_from_file("../config.json").unwrap();
        //let url = poc_config.source_chain_api_server_url();

        let rt = tokio::runtime::Runtime::new().unwrap();
        // let eth_client = rt
        //     .block_on(eth_common::Client::new(url, ""))
        //     .expect("failed to create eth client");
        // -------------------------------------- claimer part ----------------------------------
        // let fake_block = rt
        //     .block_on(eth_client.get_block(fake_block_just_for_rlp))
        //     .unwrap();

        let block_json = BlockCache::new("../data/block-cache", fake_block_just_for_rlp)
            .try_read()
            .unwrap();
        let fake_block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        let payload_bytes = fake_block.items()[index].payload_bytes();

        let block_json = BlockCache::new("../data/block-cache", block_number)
            .try_read()
            .unwrap();
        let block = OrderedBlock::try_create(
            block_json.chain_id.unwrap(),
            block_json.number,
            block_json.hash,
            block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
            block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
        )
        .unwrap();

        //        let block = rt.block_on(eth_client.get_block(block_number)).unwrap();
        // create rlp instance containing payload bytes
        //        let rlp = rlp::Rlp::new(&payload_bytes[..]);

        let out_of_bound_index = 1;
        // form claim id
        let claim_id = ClaimIdentifier::new(block_number, out_of_bound_index as u64);
        // form query of fields of interest to get values from prover for
        let claim_query =
            TxClaimQuery::try_from(vec![To].into_iter().collect::<HashSet<_>>()).unwrap();
        // claim object will be used to validate that fields got from prover correspond to local view of tx/rx payload
        let claim = Claim::try_create(claim_id, claim_query, payload_bytes).unwrap();
        // cairo_claim is sent by claimer to prover
        let cairo_claim = ClaimSerializable::from(&claim);

        println!(
            "{}",
            format!("CLAIMER: sending claim to prover: {cairo_claim:?}").bold()
        );
        // ----------------------- prover's part ------------------------------------------------
        let output = rt.block_on(run_prover(cairo_claim, block));
        let expected_out_of_bound_witness = 0u64;
        assert_eq!(
            Err(prover_primitives::claim::ClaimValidationError::ClaimOutOfBounds(0u64)),
            validate_proof_data(
                ETH_ATTESTATION_CHAIN_PARAMS_DEV,
                block_number,
                claim,
                output
            )
        );

        println!(
            "{}",
            format!(
                "CLAIMER: claim out of bounds, witness at {}",
                expected_out_of_bound_witness
            )
            .bold()
            .red()
        );
    }

    async fn run_prover(
        cairo_claim: ClaimSerializable,
        block: OrderedBlock,
    ) -> CairoVerifierOutput {
        use attestation_db::AttestationDB;

        let attestation_chain_params = ETH_ATTESTATION_CHAIN_PARAMS_DEV;
        let db_url = "../data/db";
        let db = attestation_db::EthAttestationJsonDB::try_create(attestation_chain_params, db_url)
            .unwrap();
        println!("{}", format!("PROVER: accessing db at: {db_url:?}").bold());
        let block_number = block.number();

        let attestation_fragment = db.get_fragment_for(block_number).unwrap();

        // change to false if you don't want to generate stone-proof and rather use output of cairo program
        let generate_stone_proof = true;
        let overwrite_existing_stone_proof = false;
        let cairo_output_or_stone_proof = crate::cairo_generate_proof(
            cairo_claim,
            &attestation_fragment,
            block,
            // checkpoints.inner(),
            generate_stone_proof,
            overwrite_existing_stone_proof,
        )
        .await
        .unwrap();

        println!("{}", "PROVER: sending output to claimer".bold());
        // -------------------------------------- claimer part ----------------------------------

        let output = match cairo_output_or_stone_proof {
            either::Left((mut stone_proof, stone_proof_dir)) => {
                crate::run_stone_verify_script(ClaimProver::script_source(), &stone_proof_dir)
                    .await
                    .unwrap();
                println!("{}", "CLAIMER: proof stone-verified".bold().green());

                stone_proof
                    .strip_off_annotations()
                    .strip_off_prover_config()
                    .strip_off_private_input();

                let stark_program_metadata_url = format!(
                    "{}/{}",
                    "../data/execution-chain",
                    StarkProgramMetadataStorage::DEFAULT_URL
                );
                let stark_program_metadata_storage =
                    StarkProgramMetadataStorage::retrieve_from_chain_sim(
                        &stark_program_metadata_url,
                    )
                    .unwrap();

                let metadata = StarkProgramAuth::authenticate(
                    &stone_proof,
                    &stark_program_metadata_storage,
                    default_stark_program_auth_hasher,
                )
                .map_err(|err| match err {
                    StarkProgramAuthError::AuthenticationFailure(h) => anyhow::anyhow!(
                        "STARK program not authenticated, got program bytecode fingerprint: {h:?}"
                    ),
                    _ => anyhow::anyhow!("{err:?}"),
                })
                .unwrap();

                println!(
                    "{}",
                    format!("CLAMER: STARK program authenticated, metadata: {metadata:?}")
                        .bold()
                        .bright_green()
                );

                let stone_proof_public_input =
                    StoneProofPublicInput::try_from(stone_proof.proof()).unwrap();

                println!(
                    "{}",
                    format!("Stone Proof Output: {:?}", stone_proof_public_input).bold()
                );

                stone_proof_public_input
            }
            either::Right(cairo_output) => cairo_output,
        };
        output
    }

    fn default_stark_program_auth_hasher(bytes: &[u8]) -> u64 {
        use std::hash::DefaultHasher;
        use std::hash::Hash;
        use std::hash::Hasher;

        let mut hasher = DefaultHasher::new();
        (bytes[..]).hash(&mut hasher);

        hasher.finish()
    }

    fn validate_proof_data<Q>(
        attestation_chain_params: AttestationChainParams,
        block: u64,
        claim: prover_primitives::claim::Claim<Q>,
        output: CairoVerifierOutput,
    ) -> Result<(), prover_primitives::claim::ClaimValidationError>
    where
        Q: prover_primitives::claim_query::ClaimQuery,
    {
        let checkpoints_path = "../data/execution-chain";
        println!(
            "{}",
            format!("CLAIMER: polling checkpoints from: {checkpoints_path:?} ...").bold()
        );
        let mut checkpoints = AttestationCheckpointsForDev::with_execution_chain_url(
            checkpoints_path,
            attestation_chain_params,
        );
        // simulate polling checkpoints from CC3 blockchain
        checkpoints.poll().unwrap();

        let checkpoint = checkpoints.inner().checkpoint_for(block).unwrap();
        assert_eq!(output.continuity_checkpoint_block_number, checkpoint.n());
        assert_eq!(&output.continuity_checkpoint_digest, checkpoint.digest());
        println!("{}", "CLAIMER: continuity verified".bold().green());

        claim.validate(&output)
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
}
