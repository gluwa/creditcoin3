use num_format::{Locale, ToFormattedString};
use serde_json::{json, Value};

use crate::{calculate_usc_and_source_chain_block_diff, NetworkTarget};

const MAX_ALLOWED_BLOCK_HEIGHT_DIFF: i128 = 50;
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

pub fn create_json_message(
    target: NetworkTarget,
    attestor_best_block_number: u64,
    eth_current_block_number: u64,
    slack_alert_group: Option<String>,
) -> (Value, Option<Value>) {
    let block_height_diff = calculate_usc_and_source_chain_block_diff(
        attestor_best_block_number,
        eth_current_block_number,
    );

    let exceeded = block_height_diff > MAX_ALLOWED_BLOCK_HEIGHT_DIFF;

    let eth_fmt = eth_current_block_number.to_formatted_string(&Locale::en);
    let att_fmt = attestor_best_block_number.to_formatted_string(&Locale::en);

    let (status_emoji, verdict) = if exceeded {
        ("❌", "Attestation block heights diff")
    } else {
        ("✅", "Attestation block heights diff")
    };

    // Single, consistent base line (only the emoji differs)
    let base_line = format!(
        "⬛ {}\n{} {}: {} ({}|{})",
        target.usc_network_name,
        status_emoji,
        verdict,
        block_height_diff.to_formatted_string(&Locale::en),
        eth_fmt,
        att_fmt
    );

    // Primary message (always present)
    let primary = slack_payload(code_block(base_line), ICON_PRIMARY);

    // Optional secondary alert (only when exceeded and a group is provided)
    let secondary = match slack_alert_group {
        Some(group) if exceeded => {
            let alert_text = format!("<@{group}> Current block difference exceeds threshold!");
            Some(slack_payload(alert_text, ICON_ALERT))
        }
        _ => None,
    };

    (primary, secondary)
}
