#[derive(Clone, PartialEq, Eq, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct Secret([u8; 64]);

impl From<[u8; 32]> for Secret {
    fn from(value: [u8; 32]) -> Self {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(zeroize::Zeroizing::new(value).as_ref());
        Self(out)
    }
}

impl From<bip39::Mnemonic> for Secret {
    fn from(value: bip39::Mnemonic) -> Self {
        let mut out = [0u8; 64];
        out.copy_from_slice(value.to_seed_normalized("").as_ref());
        Self(out)
    }
}

impl Into<[u8; 32]> for Secret {
    fn into(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.0);
        out
    }
}

impl Into<[u8; 64]> for Secret {
    fn into(self) -> [u8; 64] {
        self.0
    }
}

impl AsRef<[u8]> for Secret {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Secret").field(&"***").finish()
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}
