mod bls;

use bls_signatures::Serialize;
use blst::min_sig as blst_core;
use parity_scale_codec::Encode;
use sp_core::{ed25519, sr25519, Pair as _};

pub type PublicFor<C> = <C as CryptoScheme>::Public;

pub type SignatureFor<C> = <C as CryptoScheme>::Signature;

/// A cryptography scheme capabe of signing messages and
/// verifying signed messages
pub trait CryptoScheme {
    type Signature: Clone + Send + Sync;
    /// Capable of signing messages and deriving a public key
    type KeyPair: Send + Sync;
    type Public: Clone + Send + Sync;

    /// Sign message `data` with `keypair`
    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature;
    /// Verify `signature` of `data` against a `public` key
    fn verify(public: &PublicFor<Self>, signature: &Self::Signature, data: &[u8]) -> bool;

    /// Derive a public key
    fn public_key(keypair: &Self::KeyPair) -> Self::Public;
}

/// Sr25519 backed by `sp_core``
pub struct Sr25519;

impl CryptoScheme for Sr25519 {
    type Signature = sr25519::Signature;

    type KeyPair = sr25519::Pair;

    type Public = sr25519::Public;

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature {
        keypair.sign(data)
    }

    fn verify(public: &PublicFor<Self>, signature: &Self::Signature, data: &[u8]) -> bool {
        sr25519::Pair::verify(signature, data, public)
    }

    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        keypair.public()
    }
}

/// BLS backed by `bls_signatures`
pub struct Bls;

#[repr(transparent)]
#[derive(Clone, Debug)]
pub struct WrapEncode<T>(pub T);

impl<T> From<T> for WrapEncode<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> AsRef<T> for WrapEncode<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl Encode for WrapEncode<bls_signatures::Signature> {
    fn encode_to<T: parity_scale_codec::Output + ?Sized>(&self, dest: &mut T) {
        self.0.as_bytes().encode_to(dest)
    }
}

impl CryptoScheme for Bls {
    type Signature = WrapEncode<bls_signatures::Signature>;

    type KeyPair = bls_signatures::PrivateKey;

    type Public = bls_signatures::PublicKey;

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature {
        WrapEncode(keypair.sign(data))
    }

    fn verify(
        public: &PublicFor<Self>,
        WrapEncode(signature): &Self::Signature,
        data: &[u8],
    ) -> bool {
        public.verify(signature.clone(), data)
    }

    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        keypair.public_key()
    }
}

/// Ed25519 backed by `sp_core`
pub struct Ed25519;

impl CryptoScheme for Ed25519 {
    type Signature = ed25519::Signature;

    type KeyPair = ed25519::Pair;

    type Public = ed25519::Public;

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature {
        keypair.sign(data)
    }

    fn verify(public: &PublicFor<Self>, signature: &Self::Signature, data: &[u8]) -> bool {
        ed25519::Pair::verify(signature, data, public)
    }

    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        keypair.public()
    }
}

/// A cryptography scheme capable of creating and verifying aggregate signatures
pub trait AggregatableScheme: CryptoScheme {
    fn make_aggregate(signatures: &[Self::Signature]) -> Self::Signature;

    fn aggregate_verify(
        publics: &[PublicFor<Self>],
        signature: &Self::Signature,
        data: &[u8],
    ) -> bool;
}

impl AggregatableScheme for Bls {
    fn make_aggregate(signatures: &[Self::Signature]) -> Self::Signature {
        let signatures: Vec<_> = signatures
            .into_iter()
            .map(|WrapEncode(sig)| sig.clone())
            .collect();
        WrapEncode(bls_signatures::aggregate(&signatures).unwrap())
    }

    fn aggregate_verify(
        publics: &[PublicFor<Self>],
        signature: &Self::Signature,
        data: &[u8],
    ) -> bool {
        let hash = bls_signatures::hash(&data);
        let hashes = vec![hash; publics.len()];
        bls::bls_aggregate_verify(signature.as_ref(), &hashes, publics)
    }
}

/// BLS backed by `bls_on_arkworks`
pub struct BlsArkworks;

impl CryptoScheme for BlsArkworks {
    type KeyPair = bls_on_arkworks::types::SecretKey;

    type Signature = bls_on_arkworks::types::Signature;

    type Public = bls_on_arkworks::types::PublicKey;

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature {
        bls_on_arkworks::sign(
            keypair.clone(),
            &data.to_vec(),
            &bls_on_arkworks::DST_ETHEREUM.as_bytes().to_vec(),
        )
            .unwrap()
    }

    fn verify(public: &PublicFor<Self>, signature: &Self::Signature, data: &[u8]) -> bool {
        bls_on_arkworks::verify(
            public,
            &data.to_vec(),
            signature,
            &bls_on_arkworks::DST_ETHEREUM.as_bytes().to_vec(),
        )
    }
    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        bls_on_arkworks::sk_to_pk(keypair.clone())
    }
}

impl AggregatableScheme for BlsArkworks {
    fn make_aggregate(signatures: &[Self::Signature]) -> Self::Signature {
        bls_on_arkworks::aggregate(signatures).unwrap()
    }

    fn aggregate_verify(
        publics: &[PublicFor<Self>],
        signature: &Self::Signature,
        data: &[u8],
    ) -> bool {
        bls_on_arkworks::aggregate_verify(
            publics.to_vec(),
            vec![data.to_vec(); publics.len()],
            signature,
            &bls_on_arkworks::DST_ETHEREUM.as_bytes().to_vec(),
        )
    }
}

pub struct BlsBlst;

impl CryptoScheme for BlsBlst {
    type KeyPair = blst_core::SecretKey;

    type Public = blst_core::PublicKey;

    type Signature = blst_core::Signature;

    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        keypair.sk_to_pk()
    }

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature {
        keypair.sign(data, bls::DST, b"")
    }

    fn verify(public: &PublicFor<Self>, signature: &Self::Signature, data: &[u8]) -> bool {
        signature.verify(true, data, bls::DST, b"", public, true) == blst::BLST_ERROR::BLST_SUCCESS
    }
}

impl AggregatableScheme for BlsBlst {
    fn aggregate_verify(
        publics: &[PublicFor<Self>],
        signature: &Self::Signature,
        data: &[u8],
    ) -> bool {
        let publics = publics.iter().collect::<Vec<_>>();
        signature.fast_aggregate_verify(true, data, bls::DST, &publics)
            == blst::BLST_ERROR::BLST_SUCCESS
    }

    fn make_aggregate(signatures: &[Self::Signature]) -> Self::Signature {
        let signatures = signatures.iter().collect::<Vec<_>>();
        blst_core::AggregateSignature::aggregate(&signatures, true)
            .unwrap()
            .to_signature()
    }
}