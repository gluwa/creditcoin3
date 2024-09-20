pub mod attestation;
pub mod attestationcheckpoint;
pub mod blockwithdigest;
pub mod db;
pub mod schema;

#[must_use]
pub fn convert(num: u64) -> i64 {
    i64::from_ne_bytes(num.to_ne_bytes())
}
