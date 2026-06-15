fn main() {
    // Path is relative to the crate root
    println!("cargo::rerun-if-changed=contracts/block_prover.json");
}
