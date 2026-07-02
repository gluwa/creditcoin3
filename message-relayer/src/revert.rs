//! Node-agnostic classification of EVM revert errors.
//!
//! Execution clients word a revert differently: geth-style nodes say `execution reverted`,
//! Creditcoin's EVM RPC says `VM Exception while processing transaction: revert, data: "0x…"`,
//! and neither reliably decodes custom-error *names*. Matching decoded names alone therefore
//! misclassifies deterministic reverts as transient failures and retries them forever (the exact
//! bug behind the ack submitter's infinite `MessageDoesNotRequireAck` loop).
//!
//! The helpers here extract the raw 4-byte custom-error **selector** from the revert data and
//! detect revert phrasing across node dialects, so callers can classify contract reverts without
//! depending on any one node's error format. Compare selectors against the `SolError::SELECTOR`
//! constants from [`write_ability::abi`] rather than hand-computed hex.

/// Extract the 4-byte custom-error selector from a revert error string of the form
/// `… revert, data: "0x2f28bb55…"`. Anchored on the `data` field so an address or hash appearing
/// earlier in the message cannot be mistaken for a selector.
#[must_use]
pub fn revert_selector(s: &str) -> Option<[u8; 4]> {
    let data_at = s.find("data")?;
    let hex_at = s[data_at..].find("0x")? + data_at + 2;
    let hex: String = s[hex_at..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .take(8)
        .collect();
    if hex.len() < 8 {
        return None;
    }
    let mut sel = [0u8; 4];
    hex::decode_to_slice(hex.to_ascii_lowercase(), &mut sel).ok()?;
    Some(sel)
}

/// Whether the error carries `sel` as its revert selector.
#[must_use]
pub fn has_selector(s: &str, sel: [u8; 4]) -> bool {
    revert_selector(s) == Some(sel)
}

/// Whether the error string reads as a **deterministic contract revert** (any node dialect), as
/// opposed to a transport / infrastructure failure (connection refused, timeout, nonce, funds).
/// A revert observed at send / gas-estimation time re-verts identically on retry, so callers
/// should treat it as permanent for the transaction at hand.
#[must_use]
pub fn is_revert(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("execution reverted")
        || lower.contains("vm exception while processing transaction")
        || lower.contains("transaction reverted")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CC_STYLE: &str = "server returned an error response: error code -32603: VM Exception \
         while processing transaction: revert, data: \
         \"0x2f28bb55c8e0b2db4217508f44fb2d148bd9fab3c94e876a56a3fdbcf71f17570ecbe54c\"";

    #[test]
    fn selector_extraction() {
        assert_eq!(revert_selector(CC_STYLE), Some([0x2f, 0x28, 0xbb, 0x55]));
        // Anchored on `data`: an address earlier in the message is not mistaken for a selector.
        let with_addr =
            "call to 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef reverted, data: \"0x12345678\"";
        assert_eq!(revert_selector(with_addr), Some([0x12, 0x34, 0x56, 0x78]));
        assert_eq!(revert_selector("connection refused"), None);
        assert_eq!(revert_selector("revert, data: \"0xabc\""), None); // too short
    }

    #[test]
    fn revert_phrasing_across_dialects() {
        assert!(is_revert(CC_STYLE));
        assert!(is_revert("execution reverted: Already validated"));
        assert!(is_revert("transaction reverted on-chain"));
        assert!(!is_revert("error sending request: connection refused"));
        assert!(!is_revert("error code -32000: insufficient funds for gas"));
    }

    #[test]
    fn selector_match() {
        assert!(has_selector(CC_STYLE, [0x2f, 0x28, 0xbb, 0x55]));
        assert!(!has_selector(CC_STYLE, [0x33, 0x70, 0x4b, 0x28]));
    }
}
