use alloy::sol;
use eth::evm::prover::compute_current_prover_bytecode_hash;

sol! {
    #[sol(rpc)]
    CreditcoinPublicProver,
    "contracts/prover.json",
}

fn main() {
    let current_hash = compute_current_prover_bytecode_hash();
    println!("Prover.sol bytecode hash: {current_hash:?}");
}
