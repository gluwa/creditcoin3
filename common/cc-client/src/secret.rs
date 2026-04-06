#[derive(Clone, PartialEq, Eq)]
pub enum Secret {
    Mnemonic(bip39::Mnemonic),
    Seed([u8; 32]),
    Uri(String),
}

impl Secret {
    #[must_use]
    pub fn private_key(&self) -> subxt_signer::SecretString {
        match self {
            Self::Mnemonic(mnemonic) => mnemonic.to_string().into(),
            Self::Seed(seed) => format!("0x{}", hex::encode(seed)).into(),
            Self::Uri(uri) => uri.clone().into(),
        }
    }
}

impl From<bip39::Mnemonic> for Secret {
    fn from(value: bip39::Mnemonic) -> Self {
        Self::Mnemonic(value)
    }
}

impl From<[u8; 32]> for Secret {
    fn from(value: [u8; 32]) -> Self {
        Self::Seed(value)
    }
}

impl std::str::FromStr for Secret {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.starts_with("//") {
            s.parse::<subxt_signer::SecretUri>()
                .map_err(anyhow::Error::msg)
                .map(|_| Self::Uri(s.to_string()))
        } else if let Some(hex) = s.strip_prefix("0x") {
            anyhow::ensure!(
                hex.len() == 64,
                "Invalid hex seed length, expected 64 but only got {} characters",
                hex.len()
            );

            anyhow::ensure!(
                hex.chars().all(|c| c.is_ascii_hexdigit()),
                "Invalid hex seed contains non-hex digits"
            );

            let mut seed = [0; 32];
            hex::decode_to_slice(hex, &mut seed).map_err(anyhow::Error::msg)?;

            anyhow::Ok(Self::Seed(seed))
        } else {
            bip39::Mnemonic::from_str(s)
                .map_err(anyhow::Error::msg)
                .map(Self::Mnemonic)
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

impl TryInto<[u8; 32]> for Secret {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<[u8; 32], Self::Error> {
        use std::str::FromStr as _;

        match self {
            Self::Mnemonic(mnemonic) => {
                let data = mnemonic.to_seed_normalized("");
                let mut arr = [0; 32];
                arr.copy_from_slice(&data[..32]);
                Ok(arr)
            }
            Self::Seed(seed) => Ok(seed),
            Self::Uri(uri) => {
                subxt_signer::ecdsa::Keypair::from_uri(&subxt_signer::SecretUri::from_str(&uri)?)
                    .map_err(anyhow::Error::msg)
                    .map(|keypair| keypair.secret_key())
            }
        }
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::Seed([0; 32])
    }
}

impl zeroize::Zeroize for Secret {
    fn zeroize(&mut self) {
        *self = Self::Seed([0; 32]);
    }
}

impl zeroize::ZeroizeOnDrop for Secret {}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(***)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(***)")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn account_id_parity() {
        use secrecy::ExposeSecret as _;
        use std::str::FromStr as _;

        let secret = Secret::from_str("//Alice").unwrap();
        let uri = subxt_signer::SecretUri::from_str(secret.private_key().expose_secret()).unwrap();
        let keypair = subxt_signer::sr25519::Keypair::from_uri(&uri).unwrap();
        let account_id = subxt::utils::AccountId32(keypair.public_key().0);

        let uri_expected = subxt_signer::SecretUri::from_str("//Alice").unwrap();
        let keypair_expected = subxt_signer::sr25519::Keypair::from_uri(&uri_expected).unwrap();
        let account_id_expected = subxt::utils::AccountId32(keypair_expected.public_key().0);

        assert_eq!(account_id, account_id_expected);
    }
}
