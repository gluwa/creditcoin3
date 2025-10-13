use crate::{attestation_check_result::AttestationCheckResult, SupportedChainInfo};
use anyhow::{anyhow, Result};
use num_format::{Locale, ToFormattedString};
use serde_json::{json, Value};
use tracing::error;

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

fn alert_reasons(
    exceeded: bool,
    header_hash_match: bool,
    roots_match: bool,
    checkpoint_creation_in_range: bool,
    unelected_attestors_is_empty: bool,
) -> Vec<&'static str> {
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
    if !checkpoint_creation_in_range {
        reasons.push("Last checkpoint creation out of range!");
    }
    if !unelected_attestors_is_empty {
        reasons.push("Found unelected attestors in signed attestors!");
    }
    reasons
}

fn format_slack_alert_id_starts_with(group: &str) -> Result<String> {
    if group.starts_with('U') {
        Ok(format!("<@{group}>"))
    } else if group.starts_with('S') {
        Ok(format!("<!subteam^{group}>"))
    } else {
        Err(anyhow!("Unexpected Slack ID: {group}"))
    }
}

pub fn create_json_message(
    supported_chain_info: &SupportedChainInfo,
    attestation_check_result: AttestationCheckResult,
    usc_network_name: &str,
    slack_alert_group: &Option<String>,
) -> (Value, Option<Value>) {
    let eth_fmt = attestation_check_result
        .ethereum_block_info
        .latest_ethereum_block_number
        .to_formatted_string(&Locale::en);
    let att_fmt = attestation_check_result
        .attestation_info
        .attestor_best_block_number
        .to_formatted_string(&Locale::en);

    let (attestation_height_status_emoji, attestation_height_verdict) =
        if attestation_check_result.is_block_height_exceeded() {
            ("❌", "Attestation block heights diff")
        } else {
            ("✅", "Attestation block heights diff")
        };

    let (
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_emoji,
        attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_verdict,
    ) = if attestation_check_result.header_hash_matches() {
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

    let (roots_match_emoji, roots_match_verdict) = if attestation_check_result.merkle_roots_match()
    {
        ("✅", "Calculated merkle root matches attestation root")
    } else {
        (
            "❌",
            "Calculated merkle root does not match attestation root",
        )
    };

    let (checkpoint_creation_in_range_emoji, checkpoint_creation_in_range_verdict) =
        if attestation_check_result.is_checkpoint_in_range() {
            ("✅", "Last checkpoint creation is within checkpoint range")
        } else {
            ("❌", "Last checkpoint creation is outside checkpoint range")
        };

    let (
        unelected_attestors_res,
        all_signed_attestors_are_elected_emoji,
        all_signed_attestors_are_elected_verdict,
    ) = {
        if attestation_check_result.maybe_elected_attestors.is_some() {
            let unelected_attestors = attestation_check_result.get_unelected_attestors();
            if unelected_attestors.is_empty() {
                (
                    unelected_attestors,
                    "✅",
                    "All signed attestors are elected",
                )
            } else {
                (
                    unelected_attestors,
                    "❌",
                    "Found unelected attestors in signed attestors",
                )
            }
        } else {
            (Vec::new(), "⚪", "No elected attestors data")
        }
    };

    let checkpoint_interval_diff = attestation_check_result
        .check_point_created_in_range_checker
        .latest_ethereum_block_number
        .saturating_sub(
            attestation_check_result
                .check_point_created_in_range_checker
                .last_checkpoint_block_number,
        );

    let on_chain_network_details = format!(
        "[{} - {}]",
        supported_chain_info.chain_name, supported_chain_info.chain_id
    );

    // Single, consistent base line (only the emoji differs)
    let base_line = format!(
        "{} ⬛ {}\n{} {}: {} ({}|{})\n\
        {} {}: ({}|{})\n\
        {} {}: ({}|{})\n\
        {} {}: {} ({}|{})\n\
        {} {}: {} ({}|{})",
        on_chain_network_details,
        usc_network_name,
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
            .ethereum_block_info
            .fetched_ethereum_block_number_by_hash
            .map(|n| n.to_formatted_string(&Locale::en))
            .unwrap_or_else(|| "N/A".to_string()),
        attestation_check_result
            .attestation_info
            .attestor_best_block_number
            .to_formatted_string(&Locale::en),
        roots_match_emoji,
        roots_match_verdict,
        attestation_check_result
            .ethereum_block_info
            .calculated_ethereum_block_merkle_root,
        attestation_check_result
            .attestation_info
            .attestation_merkle_root,
        checkpoint_creation_in_range_emoji,
        checkpoint_creation_in_range_verdict,
        checkpoint_interval_diff.to_formatted_string(&Locale::en),
        attestation_check_result
            .check_point_created_in_range_checker
            .latest_ethereum_block_number
            .to_formatted_string(&Locale::en),
        attestation_check_result
            .check_point_created_in_range_checker
            .last_checkpoint_block_number
            .to_formatted_string(&Locale::en),
        all_signed_attestors_are_elected_emoji,
        all_signed_attestors_are_elected_verdict,
        unelected_attestors_res
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<String>>()
            .join(", "),
        attestation_check_result
            .attestation_info
            .signed_attestation
            .attestors
            .len()
            .to_formatted_string(&Locale::en),
        attestation_check_result
            .maybe_elected_attestors
            .as_ref()
            .map(|e| e.len().to_formatted_string(&Locale::en))
            .unwrap_or_else(|| "N/A".to_string())
    );

    // Primary message (always present)
    let primary = slack_payload(code_block(base_line), ICON_PRIMARY);

    // Optional secondary alert (only when exceeded and a group is provided)
    let secondary = match slack_alert_group {
        Some(group)
            if attestation_check_result.is_block_height_exceeded()
                || !attestation_check_result.header_hash_matches()
                || !attestation_check_result.merkle_roots_match()
                || !attestation_check_result.is_checkpoint_in_range()
                || !attestation_check_result
                    .get_unelected_attestors()
                    .is_empty() =>
        {
            let reasons = alert_reasons(
                attestation_check_result.is_block_height_exceeded(),
                attestation_check_result.header_hash_matches(),
                attestation_check_result.merkle_roots_match(),
                attestation_check_result.is_checkpoint_in_range(),
                attestation_check_result
                    .get_unelected_attestors()
                    .is_empty(),
            );

            let slack_alert_id = match format_slack_alert_id_starts_with(group) {
                Ok(id) => id,
                Err(e) => {
                    error!("Failed to format Slack alert ID: {e}");
                    return (primary, None);
                }
            };

            let alert_text = format!(
                "{slack_alert_id} {}\n{}",
                "The following issues were detected:",
                reasons.join("\n- ")
            );
            Some(slack_payload(alert_text, ICON_ALERT))
        }
        _ => None,
    };

    (primary, secondary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_user_id_with_u_prefix() {
        let result = format_slack_alert_id_starts_with("U123456").unwrap();
        assert_eq!(result, "<@U123456>");
    }

    #[test]
    fn formats_subteam_id_with_s_prefix() {
        let result = format_slack_alert_id_starts_with("S123456").unwrap();
        assert_eq!(result, "<!subteam^S123456>");
    }

    #[test]
    fn returns_error_for_unexpected_prefix() {
        let result = format_slack_alert_id_starts_with("X123456");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Unexpected Slack ID: X123456"
        );
    }
}
