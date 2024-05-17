pub use bls_signatures::{PublicKey, Serialize as BlsSerialize};

use sp_std::vec;
use sp_std::vec::Vec;

use parity_scale_codec::{Decode, Encode};
use scale_info::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type PublicFor<C> = <C as CryptoScheme>::Public;

pub type SignatureFor<C> = <C as CryptoScheme>::Signature;

pub trait CryptoScheme {
    type Signature: Clone + Send + Sync;
    type KeyPair: Send + Sync;
    type Public: Clone + Send + Sync;

    fn sign(keypair: &Self::KeyPair, data: &[u8]) -> Self::Signature;

    fn verify(public: &Self::Public, signature: &Self::Signature, data: &[u8]) -> bool;

    fn public_key(keypair: &Self::KeyPair) -> Self::Public;
}

/// BLS backed by `bls_signatures`
pub struct Bls;

#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
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

impl Decode for WrapEncode<bls_signatures::Signature> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        let bytes = Vec::<u8>::decode(input)?;
        let signature = bls_signatures::Signature::from_bytes(&bytes)
            .map_err(|_| parity_scale_codec::Error::from("Invalid BLS signature"))?;
        Ok(WrapEncode(signature))
    }
}

impl Serialize for WrapEncode<bls_signatures::Signature> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_bytes().serialize(serializer)
    }
}

impl<'a> Deserialize<'a> for WrapEncode<bls_signatures::Signature> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        let signature = bls_signatures::Signature::from_bytes(&bytes)
            .map_err(|_| serde::de::Error::custom("Invalid BLS signature"))?;
        Ok(Self(signature))
    }
}

impl TypeInfo for WrapEncode<bls_signatures::Signature> {
    type Identity = Self;

    fn type_info() -> Type {
        todo!("Implement TypeInfo for WrapEncode<bls_signatures::Signature>")
        // Type::builder()
        //     .path("WrapEncode")
        //     .type_params(vec![Type::builder().path("bls_signatures::Signature").build()])
        //     .variant(Type::builder().path("WrapEncode").build())
        //     .build()
    }
}
//
// impl Encode for WrapEncode<PublicKey> {
//     fn encode_to<T: parity_scale_codec::Output + ?Sized>(&self, dest: &mut T) {
//         self.0.as_bytes().encode_to(dest)
//     }
// }
//
// impl Decode for WrapEncode<PublicKey> {
//     fn decode<I: parity_scale_codec::Input>(
//         input: &mut I,
//     ) -> Result<Self, parity_scale_codec::Error> {
//         let bytes = Vec::<u8>::decode(input)?;
//         let public_key = PublicKey::from_bytes(&bytes)
//             .map_err(|_| parity_scale_codec::Error::from("Invalid BLS public key"))?;
//         Ok(WrapEncode(public_key))
//     }
// }
//
// impl Serialize for WrapEncode<PublicKey> {
//     fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
//         self.0.as_bytes().serialize(serializer)
//     }
// }
//
// impl<'a> Deserialize<'a> for WrapEncode<PublicKey> {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: Deserializer<'a>,
//     {
//         let bytes = Vec::<u8>::deserialize(deserializer)?;
//         let public_key = PublicKey::from_bytes(&bytes)
//             .map_err(|_| serde::de::Error::custom("Invalid BLS public key"))?;
//         Ok(Self(public_key))
//     }
// }
//
// impl TypeInfo for WrapEncode<PublicKey> {
//     type Identity = Self;
//
//     fn type_info() -> Type {
//        todo!("Implement TypeInfo for WrapEncode<bls_signatures::PublicKey>")
// //         Type::builder()
// //             .path("WrapEncode")
// //             .type_params(vec![Type::builder().path("bls_signatures::PublicKey").build()])
// //             .variant(Type::builder().path("WrapEncode").build())
// //             .build()
//     }
// }

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
        public.verify(*signature, data)
    }

    fn public_key(keypair: &Self::KeyPair) -> Self::Public {
        keypair.public_key()
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
        let signatures: Vec<_> = signatures.iter().map(|WrapEncode(sig)| *sig).collect();
        WrapEncode(bls_signatures::aggregate(&signatures).unwrap())
    }

    fn aggregate_verify(
        publics: &[PublicFor<Self>],
        signature: &Self::Signature,
        data: &[u8],
    ) -> bool {
        let hash = bls_signatures::hash(data);
        let hashes = vec![hash; publics.len()];
        bls_signatures::verify(signature.as_ref(), &hashes, publics)
    }
}
