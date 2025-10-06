use attestor_primitives::SignedAttestation;

use crate::NetworkTarget;
use ethers::types::U64;
use num_format::{Locale, ToFormattedString};
use serde_json::{json, Value};
use sp_core::H256;
use subxt::utils::AccountId32;

const USERNAME: &str = "usc-audit-automation";
const ICON_PRIMARY: &str = ":shield:";
const ICON_ALERT: &str = ":rotating_light:";
const CODE_FENCE: &str = "```";

fn code_block(s: impl AsRef<str>) -> String {
    format!("{CODE_FENCE}{}{CODE_FENCE}", s.as_ref())
}

fn slack_payload(text: impl Into<String>, icon: &str) -> Value {
    json!({
        "username": USERNAME,
        "icon_emoji": icon,
        "text": text.into()
    })
}

fn alert_reasons(exceeded: bool, header_hash_match: bool, roots_match: bool) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if exceeded {
        reasons.push("Current block difference exceeds threshold!");
    }
    if !header_hash_match {
        reasons.push("Attestation header hash does not match correct Ethereum block!");
    }
    if !roots_match {
        reasons.push("Calculated merkle root does not match attestation root!");
    }
    reasons
}

pub fn create_json_message(
    target: NetworkTarget,
    latest_signed_attestation: SignedAttestation<H256, AccountId32>,
    latest_ethereum_block_number: u64,
    calculated_ethereum_block_merkle_root: String,
    fetched_ethereum_block_number_by_hash: Option<U64>,
    slack_alert_group: Option<String>,
) -> (Value, Option<Value>) {
    let attestation_check_result =
        crate::attestation_check_result::compute_attestation_check_result(
            &latest_signed_attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
        );

    let eth_fmt = attestation_check_result
        .latest_ethereum_block_number
        .to_formatted_string(&Locale::en);
    let att_fmt = attestation_check_result
        .attestor_best_block_number
        .to_formatted_string(&Locale::en);

    let (attestation_height_status_emoji, attestation_height_verdict) =
        if attestation_check_result.block_height_exceeded {
            ("❌", "Attestation block heights diff")
        } else {
            ("✅", "Attestation block heights diff")
        };

    let (
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_emoji,
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_verdict,
    ) = if attestation_check_result.header_hash_matches {
        (
            "✅",
            "Attestation header hash matches correct Ethereum block",
        )
    } else {
        (
            "❌",
            "Attestation header hash does not match correct Ethereum block",
        )
    };

    let (roots_match_emoji, roots_match_verdict) = if attestation_check_result.merkle_roots_match {
        ("✅", "Calculated merkle root matches attestation root")
    } else {
        (
            "❌",
            "Calculated merkle root does not match attestation root",
        )
    };

    // Single, consistent base line (only the emoji differs)
    let base_line = format!(
        "⬛ {}\n{} {}: {} ({}|{})\n\
        {} {}: ({}|{})\n\
        {} {}: ({}|{})",
        target.usc_network_name,
        attestation_height_status_emoji,
        attestation_height_verdict,
        attestation_check_result
            .block_height_diff
            .to_formatted_string(&Locale::en),
        eth_fmt,
        att_fmt,
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_emoji,
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_verdict,
        attestation_check_result
            .fetched_ethereum_block_number_by_hash
            .map(|n| n.to_formatted_string(&Locale::en))
            .unwrap_or_else(|| "N/A".to_string()),
        attestation_check_result
            .attestor_best_block_number
            .to_formatted_string(&Locale::en),
        roots_match_emoji,
        roots_match_verdict,
        attestation_check_result.calculated_ethereum_block_merkle_root,
        attestation_check_result.attestation_merkle_root,
    );

    // Primary message (always present)
    let primary = slack_payload(code_block(base_line), ICON_PRIMARY);

    // Optional secondary alert (only when exceeded and a group is provided)
    let secondary = match slack_alert_group {
        Some(group)
            if attestation_check_result.block_height_exceeded
                || !attestation_check_result.header_hash_matches
                || !attestation_check_result.merkle_roots_match =>
        {
            let reasons = alert_reasons(
                attestation_check_result.block_height_exceeded,
                attestation_check_result.header_hash_matches,
                attestation_check_result.merkle_roots_match,
            );

            let alert_text = format!(
                "<!subteam^{group}> {}\n{}",
                "The following issues were detected:",
                reasons.join("\n- ")
            );
            Some(slack_payload(alert_text, ICON_ALERT))
        }
        _ => None,
    };

    (primary, secondary)
}
