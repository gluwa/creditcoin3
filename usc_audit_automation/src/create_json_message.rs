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

struct AlertConditions {
    pub exceeded: bool,
    pub header_hash_match: bool,
    pub _roots_match: bool,
    pub checkpoint_created_within_range: bool,
    pub elected_attestors_found: bool,
    pub attestation_found_in_graphql: bool,
    pub roots_match_in_graphql: bool,
    pub prev_digest_found_in_graphql: bool,
    pub digest_match_last_attested_digest_in_graphql: bool,
    pub last_attestation_digest_found_in_graphql: bool,
}

fn alert_reasons(conditions: &AlertConditions) -> Vec<&'static str> {
    let mut reasons = Vec::new();

    if conditions.exceeded {
        reasons.push("Current block difference exceeds threshold!");
    }
    if !conditions.header_hash_match {
        reasons.push("Attestation header hash does not match correct Ethereum block!");
    }
    // Re-enable when rpc runtimes are upgraded
    // if !conditions.roots_match {
    //     reasons.push("Calculated merkle root does not match attestation root!");
    // }
    if !conditions.checkpoint_created_within_range {
        reasons.push("Last checkpoint creation out of range!");
    }
    if !conditions.elected_attestors_found {
        reasons.push("Found unelected attestors in signed attestors!");
    }
    if !conditions.attestation_found_in_graphql {
        reasons.push("Last checkpoint number not found in GraphQL!");
    }
    if !conditions.roots_match_in_graphql {
        reasons.push("Last attestation header number not found in GraphQL!");
    }
    if !conditions.prev_digest_found_in_graphql {
        reasons.push("Last attestation root not found in GraphQL!");
    }
    if !conditions.digest_match_last_attested_digest_in_graphql {
        reasons.push("Last attestation prev digest not found in GraphQL!");
    }
    if !conditions.last_attestation_digest_found_in_graphql {
        reasons.push("Last attestation digest not found in GraphQL!");
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

    // let (roots_match_emoji, roots_match_verdict) = if attestation_check_result.merkle_roots_match()
    // {
    //     ("✅", "Calculated merkle root matches attestation root")
    // } else {
    //     (
    //         "❌",
    //         "Calculated merkle root does not match attestation root",
    //     )
    // };

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

    let (
        last_checkpoint_number_found_in_graphql_emoji,
        last_checkpoint_number_found_in_graphql_verdict,
    ) = if attestation_check_result.last_checkpoint_header_matches_checkpoint_header_in_graphql() {
        ("✅", "Last checkpoint number found in GraphQL")
    } else {
        ("❌", "Last checkpoint number not found in GraphQL")
    };

    let (
        last_attestation_header_number_found_in_graphql_emoji,
        last_attestation_header_number_found_in_graphql_verdict,
    ) = if attestation_check_result
        .last_attestation_header_number_matches_attestation_header_number_in_graphql()
    {
        ("✅", "Last attestation header number found in GraphQL")
    } else {
        ("❌", "Last attestation header number not found in GraphQL")
    };

    let (
        last_attestation_root_found_in_graphql_emoji,
        last_attestation_root_found_in_graphql_verdict,
    ) = if attestation_check_result.last_attesation_root_matches_attestation_root_in_graphql() {
        ("✅", "Last attestation root found in GraphQL")
    } else {
        ("❌", "Last attestation root not found in GraphQL")
    };

    let (
        attestation_prev_digest_found_in_graphql_emoji,
        attestation_prev_digest_found_in_graphql_verdict,
    ) = if attestation_check_result
        .last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql()
    {
        ("✅", "Last attestation prev digest found in GraphQL")
    } else {
        ("❌", "Last attestation prev digest not found in GraphQL")
    };

    let (
        last_attestation_digest_found_in_graphql_emoji,
        last_attestation_digest_found_in_graphql_verdict,
    ) = if attestation_check_result.last_attestation_digest_matches_attestation_digest_in_graphql()
    {
        ("✅", "Last attestation digest found in GraphQL")
    } else {
        ("❌", "Last attestation digest not found in GraphQL")
    };

    // todo! get actual network details
    let on_chain_network_details = format!(
        "[{} - {}]",
        supported_chain_info.chain_name, supported_chain_info.chain_id
    );

    // compute baseline vars
    let block_height_diff_str = attestation_check_result
        .block_height_diff
        .to_formatted_string(&Locale::en);

    // fetched (via hash) and attestor best block numbers
    let fetch_eth_block_number_by_hash_str = attestation_check_result
        .ethereum_block_info
        .fetched_ethereum_block_number_by_hash
        .map(|n| n.to_formatted_string(&Locale::en))
        .unwrap_or_else(|| "N/A".to_string());
    let attestor_best_block_number_str = attestation_check_result
        .attestation_info
        .attestor_best_block_number
        .to_formatted_string(&Locale::en);

    // re-add when rpc runtimes are upgraded
    // calculated and attestation merkle roots
    // let calculated_eth_merkle_root_str = &attestation_check_result
    //     .ethereum_block_info
    //     .calculated_ethereum_block_merkle_root;
    // let attestation_merkle_root_str = &attestation_check_result
    //     .attestation_info
    //     .attestation_merkle_root;

    // checkpoint interval diff and block numbers
    let checkpoint_interval_diff_str = &attestation_check_result
        .check_point_created_in_range_checker
        .latest_ethereum_block_number
        .saturating_sub(
            attestation_check_result
                .check_point_created_in_range_checker
                .last_checkpoint_block_number,
        )
        .to_formatted_string(&Locale::en);
    let latest_ethereum_block_number_str = &attestation_check_result
        .check_point_created_in_range_checker
        .latest_ethereum_block_number
        .to_formatted_string(&Locale::en);
    let last_checkpoint_block_number_str = &attestation_check_result
        .check_point_created_in_range_checker
        .last_checkpoint_block_number
        .to_formatted_string(&Locale::en);

    // unelected attestors list and counts
    let unelected_attestors_str = unelected_attestors_res
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<String>>()
        .join(", ");
    let attestors_signed_attestors_len_str = &attestation_check_result
        .attestation_info
        .signed_attestation
        .attestors
        .len()
        .to_formatted_string(&Locale::en);
    let attestors_elected_attestors_len_str = &attestation_check_result
        .maybe_elected_attestors
        .as_ref()
        .map(|e| e.len().to_formatted_string(&Locale::en))
        .unwrap_or_else(|| "N/A".to_string());

    // last checkpoint number
    let last_checkpoint_number_found_in_graphql_str = &attestation_check_result
        .graphql_attestation_check_result
        .as_ref()
        .map(|g| {
            g.checkpoint_chain_node
                .checkpoint_number
                .parse::<u64>()
                .unwrap_or_default()
        })
        .unwrap_or_default()
        .to_formatted_string(&Locale::en);
    let last_attestation_found_in_graphql_str = &attestation_check_result
        .graphql_attestation_check_result
        .as_ref()
        .map(|g| {
            g.attestation_node
                .header_number
                .parse::<u64>()
                .unwrap_or_default()
        })
        .unwrap_or_default()
        .to_formatted_string(&Locale::en);
    let latest_attestation_block_number_str = &attestation_check_result
        .attestation_info
        .signed_attestation
        .attestation
        .header_number()
        .to_formatted_string(&Locale::en);

    // last attestation root
    let last_attestation_root_in_graphql_str = attestation_check_result
        .graphql_attestation_check_result
        .as_ref()
        .map(|g| g.attestation_node.root.clone())
        .unwrap_or("N/A".to_string());
    let last_attestation_root_str = hex::encode(
        attestation_check_result
            .attestation_info
            .signed_attestation
            .attestation
            .root
            .as_bytes(),
    );

    // last attestation prev digest
    let last_attestation_prev_digest_in_graphql_str = &attestation_check_result
        .graphql_attestation_check_result
        .as_ref()
        .map(|g| g.attestation_node.prev_digest.clone())
        .unwrap_or("N/A".to_string());
    let last_attestation_prev_digest_str = hex::encode(
        attestation_check_result
            .attestation_info
            .signed_attestation
            .attestation
            .prev_digest
            .unwrap_or_default(),
    );

    // last attestation digest
    let last_attestation_digest_in_graphql_str = &attestation_check_result
        .graphql_attestation_check_result
        .as_ref()
        .map(|g| g.attestation_node.digest.clone())
        .unwrap_or("N/A".to_string());
    let last_attestation_digest_str = hex::encode(
        attestation_check_result
            .attestation_info
            .signed_attestation
            .attestation
            .digest()
            .as_bytes(),
    );

    let network_str = format!("{on_chain_network_details} ⬛ {usc_network_name}");

    let attestation_block_height_str = format!(
        "{attestation_height_status_emoji} {attestation_height_verdict}: {block_height_diff_str} ({eth_fmt}|{att_fmt})"
    );

    let header_hash_str = format!(
        "{attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_emoji} {attestation_header_matches_correct_fetched_ethereum_block_number_by_hash_verdict}: ({fetch_eth_block_number_by_hash_str}|{attestor_best_block_number_str})"
    );

    // re-add when rpc runtimes are upgraded
    // let merkle_roots_str = format!(
    //     "{roots_match_emoji} {roots_match_verdict}: ({calculated_eth_merkle_root_str}|{attestation_merkle_root_str})"
    // );

    let checkpoint_in_range_str = format!(
        "{checkpoint_creation_in_range_emoji} {checkpoint_creation_in_range_verdict}: {checkpoint_interval_diff_str} ({latest_ethereum_block_number_str}|{last_checkpoint_block_number_str})"
    );

    let attestors_elected_str = format!(
        "{all_signed_attestors_are_elected_emoji} {all_signed_attestors_are_elected_verdict}: {unelected_attestors_str} ({attestors_signed_attestors_len_str}|{attestors_elected_attestors_len_str})"
    );

    let graphql_checkpoint_number_found_str = format!(
        "{last_checkpoint_number_found_in_graphql_emoji} {last_checkpoint_number_found_in_graphql_verdict}: ({last_checkpoint_number_found_in_graphql_str}|{last_checkpoint_block_number_str})"
    );

    let graphql_attestation_header_number_found_str = format!(
        "{last_attestation_header_number_found_in_graphql_emoji} {last_attestation_header_number_found_in_graphql_verdict}: ({last_attestation_found_in_graphql_str}|{latest_attestation_block_number_str})"
    );

    let graphql_attestation_root_found_str = format!(
        "{last_attestation_root_found_in_graphql_emoji} {last_attestation_root_found_in_graphql_verdict}: ({last_attestation_root_in_graphql_str}|{last_attestation_root_str})"
    );

    let graphql_attestation_prev_digest_found_str = format!(
        "{attestation_prev_digest_found_in_graphql_emoji} {attestation_prev_digest_found_in_graphql_verdict}: ({last_attestation_prev_digest_in_graphql_str}|{last_attestation_prev_digest_str})"
    );

    let graphl_attestation_digest_found_str = format!(
        "{last_attestation_digest_found_in_graphql_emoji} {last_attestation_digest_found_in_graphql_verdict}: ({last_attestation_digest_in_graphql_str}|{last_attestation_digest_str})"
    );

    let base_line = format!(
        "{network_str}\n\
         {attestation_block_height_str}\n\
         {header_hash_str}\n\
         {checkpoint_in_range_str}\n\
         {attestors_elected_str}\n\
         {graphql_checkpoint_number_found_str}\n\
         {graphql_attestation_header_number_found_str}\n\
         {graphql_attestation_root_found_str}\n\
         {graphql_attestation_prev_digest_found_str}\n\
         {graphl_attestation_digest_found_str}"
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
                    .is_empty()
                || !attestation_check_result
                    .last_attestation_header_number_matches_attestation_header_number_in_graphql(
                    )
                || !attestation_check_result
                    .last_checkpoint_header_matches_checkpoint_header_in_graphql()
                || !attestation_check_result
                    .last_attestation_header_number_matches_attestation_header_number_in_graphql(
                    )
                || !attestation_check_result
                    .last_attesation_root_matches_attestation_root_in_graphql()
                || !attestation_check_result
                    .last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql()
                || !attestation_check_result
                    .last_attestation_digest_matches_attestation_digest_in_graphql() =>
        {
            let alert_conditions = AlertConditions {
                exceeded: attestation_check_result.is_block_height_exceeded(),
                header_hash_match: attestation_check_result.header_hash_matches(),
                _roots_match: attestation_check_result.merkle_roots_match(),
                checkpoint_created_within_range: attestation_check_result.is_checkpoint_in_range(),
                elected_attestors_found: attestation_check_result
                    .get_unelected_attestors()
                    .is_empty(),
                attestation_found_in_graphql: attestation_check_result
                    .last_checkpoint_header_matches_checkpoint_header_in_graphql(),
                roots_match_in_graphql: attestation_check_result
                    .last_attestation_header_number_matches_attestation_header_number_in_graphql(),
                prev_digest_found_in_graphql: attestation_check_result
                    .last_attesation_root_matches_attestation_root_in_graphql(),
                digest_match_last_attested_digest_in_graphql: attestation_check_result
                    .last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql(),
                last_attestation_digest_found_in_graphql: attestation_check_result
                    .last_attestation_digest_matches_attestation_digest_in_graphql(),
            };
            let reasons = alert_reasons(&alert_conditions);

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
