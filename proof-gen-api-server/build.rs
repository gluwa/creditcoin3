fn main() {
    // rerun build if this file changes
    println!("cargo:rerun-if-changed=migrations/v1/up.sql");
    println!("cargo:rerun-if-changed=migrations/v1/down.sql");
}
