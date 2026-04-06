#[derive(Clone)]
pub struct Attestor {
    chain_key: attestor_primitives::ChainKey,

    pub(crate) keypair_subxt: subxt_signer::sr25519::Keypair,
    pub(crate) keypair_p2p: libp2p::identity::Keypair,
    pub(crate) pair: sp_core::sr25519::Pair,
    pub(crate) bls_key: bls_signatures::PrivateKey,
}

impl Attestor {
    pub fn new(
        secret: crate::secret::Secret,
        chain_key: attestor_primitives::ChainKey,
    ) -> anyhow::Result<Self> {
        use sp_core::Pair as _;

        let mut sk = secret.clone().into();

        let keypair_subxt = subxt_signer::sr25519::Keypair::from_secret_key(sk)?;
        let pair = sp_core::sr25519::Pair::from_seed(&sk);
        let bls_key = bls_signatures::PrivateKey::new(&sk);
        let keypair_p2p = libp2p::identity::Keypair::ed25519_from_bytes(&mut sk)?;

        anyhow::Ok(Self {
            chain_key,

            keypair_subxt,
            keypair_p2p,

            bls_key,
            pair,
        })
    }

    #[must_use]
    pub fn chain_key(&self) -> attestor_primitives::ChainKey {
        self.chain_key
    }

    #[must_use]
    pub fn attestor_id(&self) -> attestor_primitives::AttestorId {
        attestor_primitives::AttestorId::from_public(self.keypair_subxt.public_key().0)
    }

    #[must_use]
    pub fn account_id(&self) -> subxt::utils::AccountId32 {
        subxt::utils::AccountId32(self.keypair_subxt.public_key().0)
    }

    #[must_use]
    pub fn bls_pubkey(&self) -> attestor_primitives::BlsPublicKey {
        self.bls_key.public_key().as_affine().to_compressed()
    }

    #[must_use]
    pub fn bls_proof_of_possession(&self) -> attestor_primitives::BlsSignature {
        Into::<bls12_381::G2Affine>::into(self.bls_key.sign(self.bls_pubkey())).to_compressed()
    }

    #[must_use]
    pub fn peer_id(&self) -> libp2p::PeerId {
        libp2p::PeerId::from_public_key(&self.keypair_p2p.public())
    }

    #[must_use]
    pub fn peer_keypair(&self) -> libp2p::identity::Keypair {
        self.keypair_p2p.clone()
    }

    #[must_use]
    pub fn sign_sr25519(&self, data: &[u8]) -> subxt_signer::sr25519::Signature {
        self.keypair_subxt.sign(data)
    }

    pub fn sign_bls(&self, data: &[u8]) -> bls_signatures::Signature {
        self.bls_key.sign(data)
    }
}

impl PartialEq for Attestor {
    fn eq(&self, other: &Self) -> bool {
        self.attestor_id() == other.attestor_id()
    }
}
impl Eq for Attestor {}

impl Ord for Attestor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.attestor_id().cmp(&other.attestor_id())
    }
}

impl PartialOrd for Attestor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
