use anyhow::{Context, Result};
use clap::Parser;
use std::{env, fs, path::PathBuf};
use tracing::{debug, error};
use tracing_subscriber::EnvFilter;

use usc_audit_automation::{self, attestation_checks, SanitiesChecker, SanitiesConfigFile};

fn load_config(path: &PathBuf) -> anyhow::Result<SanitiesConfigFile> {
    // Load public config from file
    let contents = fs::read_to_string(path).context("failed to read config.toml")?;
    let mut config: SanitiesConfigFile = toml::from_str(&contents)?;

    // Load secrets from environment variables
    config.slack_webhook_url = env::var("USC_SLACK_WEBHOOK_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .context("USC_SLACK_WEBHOOK_URL must be set and non-empty")?;
    config.slack_alert_group = env::var("USC_SLACK_ALERT_GROUP")
        .ok()
        .filter(|s| !s.is_empty());
    config.usc_account_mnemonic = env::var("USC_ACCOUNT_MNEMONIC")
        .ok()
        .filter(|s| !s.is_empty())
        .context("USC_ACCOUNT_MNEMONIC must be set and non-empty")?;

    // Merge API keys from environment into RPC providers
    // Only load keys for providers that are actually configured
    for provider in &mut config.rpc_providers {
        // Replace API key if None or empty string
        let needs_replacement = match &provider.api_key {
            None => true,
            Some(key) => key.is_empty(),
        };

        if needs_replacement {
            let env_var = match provider.name.to_lowercase().as_str() {
                "infura" => "USC_INFURA_API_KEY",
                "alchemy" => "USC_ALCHEMY_API_KEY",
                _ => anyhow::bail!(
                    "Unknown provider '{}' - supported providers: infura, alchemy",
                    provider.name
                ),
            };

            let key = env::var(env_var)
                .ok()
                .filter(|s| !s.is_empty())
                .with_context(|| {
                    format!(
                        "{env_var} must be set and non-empty for provider '{}'",
                        provider.name
                    )
                })?;

            provider.api_key = Some(key);
        }
    }

    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file (for local development)
    dotenv::dotenv().ok();

    // primarily check cli args
    let args = SanitiesChecker::parse(); // cli

    // load config file
    let config = match load_config(&args.config_file) {
        Ok(config) => config,
        Err(e) => {
            // Print error to stderr before tracing is initialized
            eprintln!("Failed to load config from {:?}", args.config_file);
            eprintln!("Error: {e:?}");
            eprintln!("\nFull error chain:");
            for (i, cause) in e.chain().enumerate() {
                eprintln!("  {i}: {cause}");
            }
            std::process::exit(1);
        }
    };

    // enable tracing debug logs if verbose flag is set
    let (verbose, env_filter) = if config.log_verbose {
        (true, EnvFilter::new("usc_audit_automation=debug"))
    } else {
        (false, EnvFilter::new("usc_audit_automation=info"))
    };
    tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(verbose)
        .with_env_filter(env_filter)
        .init();

    debug!("CLI args: {:?}", args);

    attestation_checks::run_attestation_sanity_checks(&config)
        .await
        .map_err(|e| {
            error!("attestation sanity checks failed: {e}");
            e
        })?;

    Ok(())
}
