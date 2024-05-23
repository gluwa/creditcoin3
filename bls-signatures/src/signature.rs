use acid_io::Write;
use core::borrow::BorrowMut;
use core::sync::atomic::{AtomicBool, Ordering};

extern crate alloc;
use alloc::vec::Vec;

#[cfg(feature = "pairing")]
use bls12_381::{
    hash_to_curve::{ExpandMsgXmd, HashToCurve},
    Bls12, G1Affine, G2Affine, G2Projective, Gt, MillerLoopResult,
};
use pairing_lib::MultiMillerLoop;

use crate::error::Error;
use crate::key::{PublicKey, Serialize};

const CSUITE: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const G2_COMPRESSED_SIZE: usize = 96;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Signature(G2Affine);

impl From<G2Projective> for Signature {
    fn from(val: G2Projective) -> Self {
        Signature(val.into())
    }
}
impl From<Signature> for G2Projective {
    fn from(val: Signature) -> Self {
        val.0.into()
    }
}

impl From<G2Affine> for Signature {
    fn from(val: G2Affine) -> Self {
        Signature(val)
    }
}

impl From<Signature> for G2Affine {
    fn from(val: Signature) -> Self {
        val.0
    }
}

impl Serialize for Signature {
    fn write_bytes(&self, dest: &mut impl Write) -> Result<(), Error> {
        dest.borrow_mut().write_all(&self.0.to_compressed())?;

        Ok(())
    }

    fn from_bytes(raw: &[u8]) -> Result<Self, Error> {
        let g2 = g2_from_slice(raw)?;
        Ok(g2.into())
    }
}

fn g2_from_slice(raw: &[u8]) -> Result<G2Affine, Error> {
    if raw.len() != G2_COMPRESSED_SIZE {
        return Err(Error::SizeMismatch);
    }

    let mut res = [0u8; G2_COMPRESSED_SIZE];
    res.copy_from_slice(raw);

    Option::from(G2Affine::from_compressed(&res)).ok_or(Error::GroupDecode)
}

/// Hash the given message, as used in the signature.
#[cfg(feature = "pairing")]
#[must_use]
pub fn hash(msg: &[u8]) -> G2Projective {
    <G2Projective as HashToCurve<ExpandMsgXmd<sha2::Sha256>>>::hash_to_curve(msg, CSUITE)
}

/// Aggregate signatures by multiplying them together.
/// Calculated by `signature = \sum_{i = 0}^n signature_i`.
///
/// # Errors
///
/// Returns an error if the input is empty.
pub fn aggregate(signatures: &[Signature]) -> Result<Signature, Error> {
    if signatures.is_empty() {
        return Err(Error::ZeroSizedInput);
    }

    let res = signatures
        .iter()
        .fold(G2Projective::identity(), |acc, signature| acc + signature.0);

    Ok(Signature(res.into()))
}

/// Verifies that the signature is the actual aggregated signature of hashes - pubkeys.
/// Calculated by `e(g1, signature) == \prod_{i = 0}^n e(pk_i, hash_i)`.
#[must_use]
pub fn verify(signature: &Signature, hashes: &[G2Projective], public_keys: &[PublicKey]) -> bool {
    if hashes.is_empty() || public_keys.is_empty() {
        return false;
    }

    let n_hashes = hashes.len();

    if n_hashes != public_keys.len() {
        return false;
    }

    // zero key & single hash should fail
    if n_hashes == 1 && public_keys[0].0.is_identity().into() {
        return false;
    }

    // Enforce that messages are distinct as a countermeasure against BLS's rogue-key attack.
    // See Section 3.1. of the IRTF's BLS signatures spec:
    // https://tools.ietf.org/html/draft-irtf-cfrg-bls-signature-02#section-3.1
    for i in 0..(n_hashes - 1) {
        for j in (i + 1)..n_hashes {
            if hashes[i] == hashes[j] {
                return false;
            }
        }
    }

    let is_valid = AtomicBool::new(true);

    let mut ml = public_keys
        .iter()
        .zip(hashes.iter())
        .map(|(pk, h)| {
            if pk.0.is_identity().into() {
                is_valid.store(false, Ordering::Relaxed);
            }
            let pk = pk.as_affine();
            let h = G2Affine::from(h).into();
            Bls12::multi_miller_loop(&[(&pk, &h)])
        })
        .fold(MillerLoopResult::default(), |acc, cur| acc + cur);

    if !is_valid.load(Ordering::Relaxed) {
        return false;
    }

    let g1_neg = -G1Affine::generator();

    ml += Bls12::multi_miller_loop(&[(&g1_neg, &signature.0.into())]);

    ml.final_exponentiation() == Gt::identity()
}

#[must_use]
pub fn verify_aggregated_signatures_on_same_message(
    signature: &Signature,
    message: &[u8],
    public_key: PublicKey,
) -> bool {
    let hash = hash(message);

    verify(signature, &[hash], &[public_key])
}

/// Verifies that the signature is the actual aggregated signature of messages - pubkeys.
/// Calculated by `e(g1, signature) == \prod_{i = 0}^n e(pk_i, hash_i)`.
#[cfg(feature = "pairing")]
#[must_use]
pub fn verify_messages(
    signature: &Signature,
    messages: &[&[u8]],
    public_keys: &[PublicKey],
) -> bool {
    let hashes: Vec<_> = messages.iter().map(|msg| hash(msg)).collect();

    verify(signature, &hashes, public_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::PrivateKey;
    #[cfg(feature = "pairing")]
    use bls12_381::Scalar;
    use ff::Field;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    use sp_std::vec;

    #[test]
    fn basic_aggregation() {
        let mut rng = ChaCha8Rng::seed_from_u64(12);

        let num_messages = 10;

        // generate private keys
        let private_keys: Vec<_> = (0..num_messages)
            .map(|_| PrivateKey::generate(&mut rng))
            .collect();

        // generate messages
        let messages: Vec<Vec<u8>> = (0..num_messages)
            .map(|_| (0..64).map(|_| rng.gen()).collect())
            .collect();

        // sign messages
        let sigs = messages
            .iter()
            .zip(&private_keys)
            .map(|(message, pk)| pk.sign(message))
            .collect::<Vec<Signature>>();

        let aggregated_signature = aggregate(&sigs).expect("failed to aggregate");

        let hashes = messages
            .iter()
            .map(|message| hash(message))
            .collect::<Vec<_>>();
        let public_keys = private_keys
            .iter()
            .map(PrivateKey::public_key)
            .collect::<Vec<_>>();

        assert!(
            verify(&aggregated_signature, &hashes, &public_keys),
            "failed to verify"
        );

        let messages = messages.iter().map(|r| &r[..]).collect::<Vec<_>>();
        assert!(verify_messages(
            &aggregated_signature,
            &messages[..],
            &public_keys
        ));
    }

    #[test]
    fn aggregation_same_messages() {
        let mut rng = ChaCha8Rng::seed_from_u64(12);

        let num_messages = 10;

        // generate private keys
        let private_keys: Vec<_> = (0..num_messages)
            .map(|_| PrivateKey::generate(&mut rng))
            .collect();

        // generate messages
        let message: Vec<u8> = (0..64).map(|_| rng.gen()).collect();

        // sign messages
        let sigs = private_keys
            .iter()
            .map(|pk| pk.sign(&message))
            .collect::<Vec<Signature>>();

        let aggregated_signature = aggregate(&sigs).expect("failed to aggregate");

        // check that equal messages can not be aggreagated
        let hashes: Vec<_> = (0..num_messages).map(|_| hash(&message)).collect();
        let public_keys = private_keys
            .iter()
            .map(PrivateKey::public_key)
            .collect::<Vec<_>>();
        assert!(
            !verify(&aggregated_signature, &hashes, &public_keys),
            "must not verify aggregate with the same messages"
        );
        let messages = vec![&message[..]; num_messages];

        assert!(!verify_messages(
            &aggregated_signature,
            &messages[..],
            &public_keys
        ));
    }

    #[test]
    fn test_zero_key() {
        let mut rng = ChaCha8Rng::seed_from_u64(12);

        // In the current iteration we expect the zero key to be valid and work.
        let zero_key: PrivateKey = Scalar::ZERO.into();
        assert!(bool::from(zero_key.public_key().0.is_identity()));

        let num_messages = 10;

        // generate private keys
        let mut private_keys: Vec<_> = (0..num_messages - 1)
            .map(|_| PrivateKey::generate(&mut rng))
            .collect();

        private_keys.push(zero_key);

        // generate messages
        let messages: Vec<Vec<u8>> = (0..num_messages)
            .map(|_| (0..64).map(|_| rng.gen()).collect())
            .collect();

        // sign messages
        let sigs = messages
            .iter()
            .zip(&private_keys)
            .map(|(message, pk)| pk.sign(message))
            .collect::<Vec<Signature>>();

        let aggregated_signature = aggregate(&sigs).expect("failed to aggregate");

        let hashes = messages
            .iter()
            .map(|message| hash(message))
            .collect::<Vec<_>>();
        let public_keys = private_keys
            .iter()
            .map(PrivateKey::public_key)
            .collect::<Vec<_>>();

        assert!(
            !verify(&aggregated_signature, &hashes, &public_keys),
            "verified with zero key"
        );

        let messages = messages.iter().map(|r| &r[..]).collect::<Vec<_>>();
        assert!(!verify_messages(
            &aggregated_signature,
            &messages[..],
            &public_keys
        ));

        // single message is rejected
        let signature = zero_key.sign(messages[0]);

        assert!(!zero_key.public_key().verify(signature, messages[0]));

        let aggregated_signature = aggregate(&[signature][..]).expect("failed to aggregate");
        assert!(!verify_messages(
            &aggregated_signature,
            &messages[..1],
            &[zero_key.public_key()][..],
        ));
    }

    #[test]
    fn test_bytes_roundtrip() {
        let mut rng = ChaCha8Rng::seed_from_u64(12);
        let sk = PrivateKey::generate(&mut rng);

        let msg = (0..64).map(|_| rng.gen()).collect::<Vec<u8>>();
        let signature = sk.sign(msg);

        let signature_bytes = signature.as_bytes();
        assert_eq!(signature_bytes.len(), 96);
        assert_eq!(Signature::from_bytes(&signature_bytes).unwrap(), signature);
    }
}
