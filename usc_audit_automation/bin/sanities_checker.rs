use anyhow::{Context, Result};
use clap::Parser;
use std::{fs, path::PathBuf};
use tracing::debug;
use tracing_subscriber::EnvFilter;

use usc_audit_automation::{self, block_height_sanity_check, SanitiesChecker, SanitiesConfigFile};

fn load_config(path: &PathBuf) -> anyhow::Result<SanitiesConfigFile> {
    let contents = fs::read_to_string(path).context("failed to read config.toml")?;
    let config: SanitiesConfigFile = toml::from_str(&contents)?;

    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    // primarily check cli args
    let args = SanitiesChecker::parse(); // cli

    // load config file
    let config = match load_config(&args.config_file) {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("Failed to load config from {:?}: {}", args.config_file, e);
            anyhow::bail!("Failed to load config");
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

    block_height_sanity_check::check_best_block_height_diff(&config).await?;

    Ok(())
}
