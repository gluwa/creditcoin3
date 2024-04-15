use bls12_381::{Bls12, G1Affine, G1Projective, G2Affine, G2Projective, Gt, MillerLoopResult};
use bls_signatures::PublicKey;
use pairing::MultiMillerLoop;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn is_identity(pk: &PublicKey) -> bool {
    G1Projective::from(pk.clone()).is_identity().into()
}

/// Basically just inlined `bls_signatures::aggregate_verify` but without the check that
/// enforces that the messages be distinct.
/// this is only secure if you prove possession of the private key for each public key
pub fn bls_aggregate_verify(
    signature: &bls_signatures::Signature,
    hashes: &[G2Projective],
    public_keys: &[PublicKey],
) -> bool {
    if hashes.is_empty() || public_keys.is_empty() {
        return false;
    }

    let n_hashes = hashes.len();

    if n_hashes != public_keys.len() {
        return false;
    }

    // zero key & single hash should fail
    if n_hashes == 1
        && G1Projective::from(public_keys[0].clone())
        .is_identity()
        .into()
    {
        return false;
    }

    let is_valid = AtomicBool::new(true);

    let mut ml = public_keys
        .par_iter()
        .zip(hashes.par_iter())
        .map(|(pk, h)| {
            if is_identity(pk) {
                is_valid.store(false, Ordering::Relaxed);
            }
            let pk = pk.as_affine();
            let h = G2Affine::from(h).into();
            Bls12::multi_miller_loop(&[(&pk, &h)])
        })
        .reduce(MillerLoopResult::default, |acc, cur| acc + cur);

    if !is_valid.load(Ordering::Relaxed) {
        return false;
    }

    let g1_neg = -G1Affine::generator();

    ml += Bls12::multi_miller_loop(&[(&g1_neg, &G2Affine::from(signature.clone()).into())]);

    ml.final_exponentiation() == Gt::identity()
}

pub const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";