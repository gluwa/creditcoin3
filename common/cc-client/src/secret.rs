#[derive(Clone, PartialEq, Eq, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct Secret([u8; 64]);

impl Secret {
    pub fn leak_private_key(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }
}

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

impl std::str::FromStr for Secret {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().strip_prefix("0x") {
            Some(hex) => {
                anyhow::ensure!(
                    hex.len() == 64,
                    "Invalid hex seed length, expected 64 but got {}",
                    hex.len()
                );

                anyhow::ensure!(
                    hex.chars().all(|c| c.is_ascii_hexdigit()),
                    "Invalid hex seed, contains non-hex digits"
                );

                let mut bytes = [0u8; 32];
                hex::decode_to_slice(hex, &mut bytes).map_err(anyhow::Error::msg)?;
                Ok(Self::from(bytes))
            }
            None => {
                let seed = bip39::Mnemonic::from_str(s).map_err(anyhow::Error::msg)?;
                Ok(Self::from(seed))
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&str>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
