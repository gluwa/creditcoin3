fn main() {
    // Path is relative to the crate root
    println!("cargo::rerun-if-changed=contracts/native_query_verifier.json");
}
