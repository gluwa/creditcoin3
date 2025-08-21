use url::Url;

const REDACTED: &str = "redacted";

pub fn sanitize_rpc_url_api_key(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };

    // Redact Infura-style /v3/<project-id>
    if let Some(segments) = parsed.path_segments() {
        let mut new_segments = Vec::new();
        let mut redact_next = false;
        for segment in segments {
            if redact_next {
                new_segments.push(REDACTED.to_string());
                redact_next = false;
            } else {
                new_segments.push(segment.to_string());

                if segment == "v2" || segment == "v3" {
                    redact_next = true;
                }
            }
        }
        parsed.set_path(&new_segments.join("/"));
    }

    // Redact sensitive query params or long tokens
    if parsed.query().is_some() {
        // collect pairs first, then rewrite
        let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();

        let mut qp = parsed.query_pairs_mut();
        qp.clear(); // erase the current query string

        for (k, v) in pairs {
            let redact = matches_ignore_ascii(&k, &["apikey", "key", "token"]) || v.len() > 20;
            qp.append_pair(&k, if redact { REDACTED } else { &v });
        }
    }

    parsed.to_string()
}

fn matches_ignore_ascii(k: &str, keys: &[&str]) -> bool {
    keys.iter().any(|x| k.eq_ignore_ascii_case(x))
}

#[cfg(test)]
mod tests {
    use super::sanitize_rpc_url_api_key;

    #[test]
    fn test_infura_redaction_with_v3() {
        let url = "https://sepolia.infura.io/v3/12345678901234567890123456789012";
        let s = sanitize_rpc_url_api_key(url);
        assert!(!s.contains("12345678901234567890123456789012"));
        assert!(s.contains("/v3/redacted"));
    }

    #[test]
    fn test_alchemy_redaction_with_v2() {
        let url = "https://arb-mainnet.g.alchemy.com/v2/12345678901234567890123456789012";
        let s = sanitize_rpc_url_api_key(url);
        assert!(!s.contains("12345678901234567890123456789012"));
        assert!(s.contains("/v2/redacted"));
    }

    #[test]
    fn test_url_should_redact_matched_query_param() {
        let url = "https://arb1.arbitrum.io/rpc?apikey=supersecretapikeythatisverylong";
        let s = sanitize_rpc_url_api_key(url);
        assert!(!s.contains("supersecretapikeythatisverylong"));
        assert!(s.contains("apikey=redacted"));
    }

    #[test]
    fn test_url_should_remain_unchanged() {
        let url = "https://polygon-rpc.com/?foo=bar";
        let s = sanitize_rpc_url_api_key(url);
        assert_eq!(s, url);
    }

    #[test]
    fn test_invalid_url_fallback() {
        let url = "not a url";
        let s = sanitize_rpc_url_api_key(url);
        assert_eq!(s, url);
    }
}
