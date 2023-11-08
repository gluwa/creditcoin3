use std::path::PathBuf;

use clap::Parser;
use generate_bags::generate_thresholds;

#[derive(Parser)]
struct Cli {
    /// Total issuance of the currency in millions
    #[arg(short, long)]
    total_issuance: u128,

    /// Minimum account balance
    #[arg(short, long, default_value_t = 8456776)]
    minimum_balance: u128,

    #[arg(long, default_value_t = 200)]
    n_bags: usize,

    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<(), std::io::Error> {
    let Cli {
        total_issuance,
        minimum_balance,
        n_bags,
        output,
    } = Cli::parse();

    let issuance_ctc = total_issuance * creditcoin_next_runtime::CTC * 1_000_000;

    println!(
        "Issuance ctc = {issuance_ctc}; factor = {}",
        (issuance_ctc / u64::MAX as u128)
    );

    generate_thresholds::<creditcoin_next_runtime::Runtime>(
        n_bags,
        &output,
        issuance_ctc,
        minimum_balance,
    )
}
