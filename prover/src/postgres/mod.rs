pub mod attestation;
pub mod attestationcheckpoint;
pub mod blockwithdigest;
pub mod cachedupto;
pub mod db;
pub mod schema;

#[must_use]
pub fn to_storage_type(num: u64) -> i64 {
    i64::from_ne_bytes(num.to_ne_bytes())
}

#[must_use]
pub fn from_storage_type(num: i64) -> u64 {
    u64::from_ne_bytes(num.to_ne_bytes())
}
