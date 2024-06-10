pub mod claim_prover;
pub mod eth;
pub mod fragment;
pub mod types;

use anyhow::anyhow;
use attestor_primitives::BlockAttestation;
use std::os::fd::AsFd;
//use claim_prover::build_prover;

use crate::claim_prover::ClaimProver;
use crate::fragment::AttestationFragment;
use crate::types::{CairoVerifierOutput, ClaimProverError};
use attestor::merkle::tree::FieldElement;
use prover_primitives::claim::{Claim, ClaimKind};
use tokio::{fs::File, io::AsyncReadExt};

const SOME_FRAGMENT_SIZE: usize = 5;

pub async fn cairo_generate_proof<Address>(
    claim: Claim<Address>,
    attestation_fragment: &AttestationFragment,
    tx_bytes: Vec<Vec<u8>>,
    rx_bytes: Vec<Vec<u8>>,
) -> anyhow::Result<()> {
    let claim_block_number: u64 = claim.block_number;
    let attestation_chain_slice = attestation_fragment.attestation_slice_for(claim_block_number, None)
        .ok_or(anyhow!("can't create attestation checkpoint slice for {} on this attestation chain ({:?}, {:?})",
            claim_block_number,
            attestation_fragment.tail().map(|att| att.header_number()),
            attestation_fragment.head().map(|att| att.header_number())))?;

    let prover = ClaimProver::build_prover(claim, attestation_chain_slice, tx_bytes, rx_bytes)
        .await
        .map(|claim_prover| {
            println!("done");
            println!("cairo0 input file {:?}", claim_prover.file_name());
            claim_prover
    })
        .map_err(|err| {
            anyhow!("{}",
                match &err {
                    ClaimProverError::AttestationFragmentMismatch(b, tail, head) =>
                        format!("can't create attestation checkpoint slice for {b} on this attestation chain ({tail:?}, {head:?})"),
                    ClaimProverError::BlockFetchFailure(msg) =>
                        format!("failure while fetching block corresponding claim: {msg}"),
                    ClaimProverError::ClaimNotIdentified =>
                        "claim was not identified in the block".to_string(),
                    ClaimProverError::ClaimNotUnique =>
                        "claim not uniquely identified in the block, refine filtering criteria".to_string(),
                    err => format!("could not build verifier: {err:?}"),
                }
            )
        })?
        .cairo_verify(true)
        .await
        .map_err(|err| anyhow!("{err:?}"))
        .map(|mut claim_prover| {
            let output = claim_prover.take_output().expect("successful verification yields output");

            println!("----- cairo verification successful -----");
            println!("cairo verification output: {:?}", output);
            claim_prover
        })
        // ToDo continuity validation at checkpoints here
        .and_then(|claim_prover|{
            if true {
                Ok(claim_prover)
            } else {
                Err(anyhow!("proof generation failed"))
            }
        })?;
    // ToDo always stone proving, make this configurable
    if true {
        println!("stone proving... will take some time");

        prover
            .stone_prove(true)
            .await
            .map(|msg| println!("{}", msg))
            .map_err(|err| anyhow!("{err:?}"))
    } else {
        Ok(())
    }
}

#[tokio::test]
async fn test_cairo_generate_proof() {
    // ToDo
    let claim = Claim {
        chain_id: 0,
        block_number: 19543674,
        tx_index: 0x11,
        from: "0xc37362927fe05aba72c533e23f97781ebb0877b7",
        to: "0x9b9647431632af44be02ddd22477ed94d14aacaa",
        kind: ClaimKind::Rx,
    };

    let att1 = BlockAttestation {
        block_number: 19543672,
        tx_root: FieldElement::from_dec_str(
            "1730029226712287283625343349648287262633652074500146618079593135643196863334".as_ref(),
        )
        .unwrap(),
        rx_root: FieldElement::from_dec_str(
            "2976310028842250931614337973419246799732187412150662372262748884712533368052".as_ref(),
        )
        .unwrap(),
        prev_digest: FieldElement::from_dec_str(
            "000000000000000000000000000000000000000000000000000000000000000000000000000".as_ref(),
        )
        .unwrap(),
        digest: FieldElement::from_dec_str(
            "957557156768970007813030806711276673390269449912169785311563311253398517646".as_ref(),
        )
        .unwrap(),
    };

    let att2 = BlockAttestation {
        block_number: 19543673,
        tx_root: FieldElement::from_dec_str(
            "2804518106394961886505830853749725749107561316450119143644615672880228111014".as_ref(),
        )
        .unwrap(),
        rx_root: FieldElement::from_dec_str(
            "2241421852074295547956850702263696450907673665495240773159235287302864374988".as_ref(),
        )
        .unwrap(),
        prev_digest: FieldElement::from_dec_str(
            "957557156768970007813030806711276673390269449912169785311563311253398517646".as_ref(),
        )
        .unwrap(),
        digest: FieldElement::from_dec_str(
            "2243274825215257874235489694730852979328209710580434206775996433564470378086".as_ref(),
        )
        .unwrap(),
    };

    let att3 = BlockAttestation {
        block_number: 19543674,
        tx_root: FieldElement::from_dec_str(
            "1650285496682882100196203453056579872474782262612983757579575523952258804399".as_ref(),
        )
        .unwrap(),
        rx_root: FieldElement::from_dec_str(
            "2774373924042063225686852017418633883649363447256155232327621780612266897946".as_ref(),
        )
        .unwrap(),
        prev_digest: FieldElement::from_dec_str(
            "2243274825215257874235489694730852979328209710580434206775996433564470378086".as_ref(),
        )
        .unwrap(),
        digest: FieldElement::from_dec_str(
            "148423544603031434156059001399389504786284405970174057774967538614785944798".as_ref(),
        )
        .unwrap(),
    };

    let att4 = BlockAttestation {
        block_number: 19543675,
        tx_root: FieldElement::from_dec_str(
            "000000000000000000000000000000000000000000000000000000000000000000000000000".as_ref(),
        )
        .unwrap(),
        rx_root: FieldElement::from_dec_str(
            "000000000000000000000000000000000000000000000000000000000000000000000000000".as_ref(),
        )
        .unwrap(),
        prev_digest: FieldElement::from_dec_str(
            "148423544603031434156059001399389504786284405970174057774967538614785944798".as_ref(),
        )
        .unwrap(),
        digest: FieldElement::from_dec_str(
            "2687230123067379987899726620028707571645047797244764298536114987985591982606".as_ref(),
        )
        .unwrap(),
    };

    let att5 = BlockAttestation {
        block_number: 19543676,
        tx_root: FieldElement::from_dec_str(
            "3518195695565040937707985852221095261407757251524320194026033337092578497374".as_ref(),
        )
        .unwrap(),
        rx_root: FieldElement::from_dec_str(
            "924256633821954093825555968433330603637463931069479457103877798059916073714".as_ref(),
        )
        .unwrap(),
        prev_digest: FieldElement::from_dec_str(
            "2687230123067379987899726620028707571645047797244764298536114987985591982606".as_ref(),
        )
        .unwrap(),
        digest: FieldElement::from_dec_str(
            "1720736962047806001433973964549945821537816635634855954453126389221365990231".as_ref(),
        )
        .unwrap(),
    };

    let attestation_fragment = AttestationFragment {
        attestations: [att1, att2, att3, att4, att5],
        len: 5,
    };

    use attestor::transaction::BlockItem;
    let tx_asd = eth::fetch_block_transactions(
        "wss://eth-mainnet.g.alchemy.com/v2/ziEK05XpthEPz4a3g1iA4iD828g6wm_e",
        19543674,
    )
    .await
    .unwrap()
    .iter()
    .map(|tx| tx.to_bytes())
    .collect::<Vec<Vec<u8>>>();
    let rx_asd = eth::fetch_block_receipts(
        "wss://eth-mainnet.g.alchemy.com/v2/ziEK05XpthEPz4a3g1iA4iD828g6wm_e",
        19543674,
    )
    .await
    .unwrap()
    .iter()
    .map(|rx| rx.to_bytes())
    .collect::<Vec<Vec<u8>>>();

    let result = cairo_generate_proof(claim, &attestation_fragment, tx_asd, rx_asd).await;
    println!("{:?}", result);
    assert!(result.is_ok());
}
