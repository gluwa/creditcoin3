//! Peer authorization via BLS proof-of-possession.
//!
//! libp2p authenticates a peer's [`PeerId`] during the Noise handshake (the `PeerId` is a
//! commitment to the peer's ed25519 key), but it does **not** know whether that peer is an
//! authorized attestor. Authorization is tracked on-chain, keyed by an attestor's account /
//! BLS key — an identity which is unrelated to the libp2p `PeerId`.
//!
//! This module bridges the two: on connection, a peer sends a [`PeerAuth`] proving it controls the
//! BLS private key of an attestor in the on-chain active set, signed over its own `PeerId`. Because
//! the signature is bound to the connection's authenticated `PeerId`, the proof cannot be replayed
//! onto another connection, so no challenge/nonce round-trip is required.
//!
//! [`PeerId`]: libp2p::PeerId

use parity_scale_codec::{Decode, Encode};

/// Domain separation tag for the proof-of-possession message.
///
/// The same BLS key is used to sign attestations (over serialized [`AttestationData`]). Prefixing
/// the signed message with a distinct, fixed tag guarantees a peer-auth signature can never be a
/// valid attestation signature (or vice versa), even if an attacker could influence the `PeerId`.
///
/// [`AttestationData`]: attestor_primitives::AttestationData
const DOMAIN: &[u8] = b"cc3-attestor-p2p-pop:v1";

/// Upper bound on an encoded auth message. A [`PeerAuth`] is 32 + 96 bytes plus a few bytes of
/// SCALE framing; the cap bounds the memory a hostile peer can make us buffer.
const MAX_AUTH_BYTES: u32 = 1024;

/// A peer's proof that it controls the BLS private key of an authorized attestor, binding that
/// on-chain identity to the libp2p [`PeerId`] of the connection it is sent on.
///
/// [`PeerId`]: libp2p::PeerId
#[derive(Clone, Debug, Encode, Decode)]
pub struct PeerAuth {
    /// The attestor account (public key bytes) claiming this connection.
    pub attestor: [u8; 32],
    /// BLS signature over [`signing_message`].
    pub signature: attestor_primitives::BlsSignature,
}

/// The message signed by an attestor's BLS key to prove ownership of a connection's `PeerId`.
///
/// `DOMAIN || chain_key || peer_id`. The `chain_key` scopes the proof to a single attestation
/// network so a proof produced for one chain cannot be reused on another.
fn signing_message(chain_key: attestor_primitives::ChainKey, peer_id: &libp2p::PeerId) -> Vec<u8> {
    let peer_bytes = peer_id.to_bytes();
    let chain_bytes = chain_key.encode();

    let mut msg = Vec::with_capacity(DOMAIN.len() + chain_bytes.len() + peer_bytes.len());
    msg.extend_from_slice(DOMAIN);
    msg.extend_from_slice(&chain_bytes);
    msg.extend_from_slice(&peer_bytes);
    msg
}

impl PeerAuth {
    /// Build our own proof of possession. This value is static for the lifetime of the node, as it
    /// depends only on our BLS key, the chain key and our own (fixed) `PeerId`.
    pub fn new(
        bls_key: &bls_signatures::PrivateKey,
        attestor: [u8; 32],
        chain_key: attestor_primitives::ChainKey,
        peer_id: &libp2p::PeerId,
    ) -> Self {
        use bls_signatures::Serialize as _;

        let signature = bls_key.sign(signing_message(chain_key, peer_id));

        let mut bytes = [0u8; 96];
        bytes.copy_from_slice(&signature.as_bytes());

        Self {
            attestor,
            signature: bytes,
        }
    }

    /// Verify this proof against a known-authorized BLS public key and the libp2p-authenticated
    /// `peer_id` of the connection it arrived on.
    ///
    /// The caller is responsible for ensuring `pubkey` corresponds to an attestor in the active set
    /// (see the [`BlsStore`]). `peer_id` **must** be the connection's authenticated peer, never a
    /// value taken from the message, otherwise the proof could be replayed.
    ///
    /// [`BlsStore`]: crate::bls::BlsStore
    pub fn verify(
        &self,
        pubkey: &bls_signatures::PublicKey,
        chain_key: attestor_primitives::ChainKey,
        peer_id: &libp2p::PeerId,
    ) -> bool {
        use bls_signatures::Serialize as _;

        let Ok(signature) = bls_signatures::Signature::from_bytes(&self.signature) else {
            return false;
        };

        pubkey.verify(signature, signing_message(chain_key, peer_id))
    }
}

/// [`request_response`] codec for the peer-auth handshake. Messages are length-prefixed SCALE
/// encodings of a [`PeerAuth`], matching the encoding used elsewhere in the attestor.
///
/// [`request_response`]: libp2p::request_response
#[derive(Clone, Default)]
pub struct AuthCodec;

#[async_trait::async_trait]
impl libp2p::request_response::Codec for AuthCodec {
    type Protocol = libp2p::StreamProtocol;
    type Request = PeerAuth;
    type Response = PeerAuth;

    async fn read_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read_scale(io).await
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Response>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read_scale(io).await
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        write_scale(io, req).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        write_scale(io, res).await
    }
}

async fn read_scale<T>(io: &mut T) -> std::io::Result<PeerAuth>
where
    T: futures::AsyncRead + Unpin + Send,
{
    use futures::AsyncReadExt as _;

    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_AUTH_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "peer auth message exceeds maximum size",
        ));
    }

    let mut buf = vec![0u8; len as usize];
    io.read_exact(&mut buf).await?;

    PeerAuth::decode(&mut buf.as_slice())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))
}

async fn write_scale<T>(io: &mut T, auth: PeerAuth) -> std::io::Result<()>
where
    T: futures::AsyncWrite + Unpin + Send,
{
    use futures::AsyncWriteExt as _;

    let bytes = auth.encode();
    io.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    io.write_all(&bytes).await?;
    io.close().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: &[u8]) -> bls_signatures::PrivateKey {
        // BLS key derivation (HKDF) requires at least 32 bytes of input keying material.
        let mut ikm = [0u8; 32];
        for (slot, byte) in ikm.iter_mut().zip(seed.iter().cycle()) {
            *slot = *byte;
        }
        bls_signatures::PrivateKey::new(ikm)
    }

    fn peer_id(seed: u8) -> libp2p::PeerId {
        let mut bytes = [seed; 32];
        let keypair =
            libp2p::identity::Keypair::ed25519_from_bytes(&mut bytes).expect("valid ed25519 seed");
        libp2p::PeerId::from_public_key(&keypair.public())
    }

    #[test]
    fn valid_proof_verifies() {
        let bls = key(b"attestor-seed");
        let chain_key = 2;
        let peer = peer_id(1);

        let auth = PeerAuth::new(&bls, [7u8; 32], chain_key, &peer);

        assert!(auth.verify(&bls.public_key(), chain_key, &peer));
    }

    #[test]
    fn proof_is_bound_to_peer_id() {
        // A proof produced for one PeerId must not verify against a different connection's PeerId,
        // otherwise a captured proof could be replayed by an impersonating peer.
        let bls = key(b"attestor-seed");
        let chain_key = 2;

        let auth = PeerAuth::new(&bls, [7u8; 32], chain_key, &peer_id(1));

        assert!(!auth.verify(&bls.public_key(), chain_key, &peer_id(2)));
    }

    #[test]
    fn proof_is_bound_to_chain_key() {
        let bls = key(b"attestor-seed");
        let peer = peer_id(1);

        let auth = PeerAuth::new(&bls, [7u8; 32], 2, &peer);

        assert!(!auth.verify(&bls.public_key(), 3, &peer));
    }

    #[test]
    fn proof_rejects_wrong_key() {
        let chain_key = 2;
        let peer = peer_id(1);

        let auth = PeerAuth::new(&key(b"attestor-seed"), [7u8; 32], chain_key, &peer);

        assert!(!auth.verify(&key(b"other-seed").public_key(), chain_key, &peer));
    }

    #[test]
    fn domain_separation_blocks_attestation_signature_reuse() {
        // A signature over a non-domain-prefixed message (e.g. an attestation) must never verify as
        // a peer-auth proof for the same key.
        let bls = key(b"attestor-seed");
        let chain_key = 2;
        let peer = peer_id(1);

        let raw = bls.sign(b"some attestation payload");
        let mut signature = [0u8; 96];
        {
            use bls_signatures::Serialize as _;
            signature.copy_from_slice(&raw.as_bytes());
        }
        let forged = PeerAuth {
            attestor: [7u8; 32],
            signature,
        };

        assert!(!forged.verify(&bls.public_key(), chain_key, &peer));
    }

    #[test]
    fn scale_round_trip() {
        let auth = PeerAuth {
            attestor: [3u8; 32],
            signature: [9u8; 96],
        };

        let encoded = auth.encode();
        let decoded = PeerAuth::decode(&mut encoded.as_slice()).expect("decodes");

        assert_eq!(decoded.attestor, auth.attestor);
        assert_eq!(decoded.signature, auth.signature);
    }
}
