//! Attestor secret: either a BIP39 mnemonic or a raw 32-byte hex seed (e.g. `0x398f...`).

use std::str::FromStr;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Secret used for the attestor identity: BIP39 mnemonic or raw 32-byte seed as hex.
/// Implements [`Zeroize`] and [`ZeroizeOnDrop`] so sensitive data is cleared on drop.
/// [`Debug`] and [`Display`] are redacted so the secret is never logged or printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub enum AttestorSecret {
    Mnemonic(bip39::Mnemonic),
    RawSeed([u8; 32]),
}

impl AttestorSecret {
    /// String suitable for Substrate `SecretUri::from_str` (mnemonic phrase or `0x` + 64 hex).
    /// Returned in [`Zeroizing`] so the string is zeroized when dropped.
    pub fn to_secret_uri_string(&self) -> Zeroizing<String> {
        let s = match self {
            AttestorSecret::Mnemonic(m) => m.to_string(),
            AttestorSecret::RawSeed(bytes) => format!("0x{}", hex::encode(bytes)),
        };
        Zeroizing::new(s)
    }

    /// First 32 bytes of the seed (for P2P keypair: mnemonic seed or raw bytes).
    /// Returned in [`Zeroizing`] so the bytes are zeroized when dropped.
    pub fn to_seed_bytes_32(&self) -> Zeroizing<[u8; 32]> {
        match self {
            AttestorSecret::Mnemonic(m) => {
                let full_seed = m.to_seed_normalized("");
                let full_seed_zeroizing = Zeroizing::new({
                    let mut arr = [0u8; 64];
                    arr.copy_from_slice(full_seed.as_ref());
                    arr
                });
                let mut out = [0u8; 32];
                out.copy_from_slice(&full_seed_zeroizing[..32]);
                Zeroizing::new(out)
            }
            AttestorSecret::RawSeed(bytes) => Zeroizing::new(*bytes),
        }
    }

    /// Bytes used for BLS key derivation (same as secret URI string as bytes).
    /// Returned in [`Zeroizing`] so the bytes are zeroized when dropped.
    pub fn to_bls_seed_bytes(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.to_secret_uri_string().as_bytes().to_vec())
    }
}

impl FromStr for AttestorSecret {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.starts_with("0x") {
            let hex = s.strip_prefix("0x").unwrap();
            if hex.len() != 64 {
                return Err(anyhow::anyhow!(
                    "invalid hex seed: expected 0x followed by 64 hex digits, got {} characters",
                    hex.len()
                ));
            }
            if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(anyhow::anyhow!(
                    "invalid hex seed: 0x prefix must be followed by 64 hex digits (0-9, a-f, A-F)"
                ));
            }
            let mut bytes = [0u8; 32];
            hex::decode_to_slice(hex, &mut bytes).map_err(anyhow::Error::msg)?;
            return Ok(AttestorSecret::RawSeed(bytes));
        }
        bip39::Mnemonic::from_str(s)
            .map(AttestorSecret::Mnemonic)
            .map_err(|e| anyhow::anyhow!("invalid mnemonic or hex seed: {e}"))
    }
}

impl std::fmt::Debug for AttestorSecret {
    /// Redacted so {:?}, dbg!(), and Debug in tracing never expose the secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestorSecret::Mnemonic(_) => f.write_str("AttestorSecret(Mnemonic(***))"),
            AttestorSecret::RawSeed(_) => f.write_str("AttestorSecret(RawSeed(***))"),
        }
    }
}

impl std::fmt::Display for AttestorSecret {
    /// Redacted to avoid leaking the secret via e.g. `format!("{}", secret)` or logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttestorSecret(***)")
    }
}

/// Wrapper struct around [`Url`] which avoids leaking RPC api keys through logs.
///
/// [`Url`]: url::Url
pub enum RpcSecret {
    /// Hides the RPC url on calls to [`Debug`] or [`Display`].
    ///
    /// [`Debug`]: std::fmt::Debug
    /// [`Display`]: std::fmt::Display
    Opaque(url::Url),
    /// Exposes the RPC url on calls to [`Debug`] or [`Display`].
    ///
    /// <div class="warning">
    ///
    /// Use this for testing purposes only! This option should not be used in environment where
    /// logs are publicly accessible, such as Github actions or other CI.
    ///
    /// </div>
    ///
    /// [`Debug`]: std::fmt::Debug
    /// [`Display`]: std::fmt::Display
    UnsafeExposed(url::Url),
}

impl RpcSecret {
    /// Creates a new masked [`RpcSecret`].
    pub fn new_opaque(url: url::Url) -> Self {
        Self::Opaque(url)
    }

    /// Creates a new [`RpcSecret`] **which exposes the underlying RPC url**.
    pub fn new_unsafe(url: url::Url) -> Self {
        Self::UnsafeExposed(url)
    }
}

impl From<RpcSecret> for url::Url {
    fn from(value: RpcSecret) -> Self {
        match value {
            RpcSecret::Opaque(url) => url,
            RpcSecret::UnsafeExposed(url) => url,
        }
    }
}

impl AsRef<url::Url> for RpcSecret {
    fn as_ref(&self) -> &url::Url {
        match self {
            Self::Opaque(url) => url,
            Self::UnsafeExposed(url) => url,
        }
    }
}

impl std::fmt::Debug for RpcSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opaque(_) => f.debug_tuple("RpcSecret").field(&"***").finish(),
            Self::UnsafeExposed(url) => f.debug_tuple("RpcSecret").field(url).finish(),
        }
    }
}

impl std::fmt::Display for RpcSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opaque(_) => write!(f, "***"),
            Self::UnsafeExposed(url) => write!(f, "{url}"),
        }
    }
}
