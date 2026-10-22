//! Export metadata without starting a node (SKIP_WASM_BUILD=1 is supported).
use parity_scale_codec::Encode;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).ok_or("expected an output path")?;
    let metadata = creditcoin3_runtime::Runtime::metadata_at_version(15)
        .ok_or("runtime does not expose metadata v15")?;
    std::fs::write(output, &*metadata)?;
    if let Some(legacy) = std::env::args().nth(2) {
        std::fs::write(legacy, creditcoin3_runtime::Runtime::metadata().encode())?;
    }
    Ok(())
}
