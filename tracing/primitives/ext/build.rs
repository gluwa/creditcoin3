fn main() {
    println!("cargo::rustc-check-cfg=cfg(substrate_runtime)");
}
