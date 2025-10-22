#![allow(clippy::missing_docs_in_private_items)]

/// Hash trait used by the Merkle tree implementation.
///
/// This trait abstracts over a hashing algorithm whose output type is provided
/// via the associated `Output` type. Implementors should ensure that:
/// - `Output::default()` represents the hash of an empty (or domain-separated) input
/// - `From<u8>` is implemented to support domain separation prefixes
///
/// The tree implementation will:
/// - Prefix leaves and internal nodes using `From<u8>` conversions
/// - Pass raw byte slices directly to `hash`
///
/// NOTE:
/// The previous `concat_then_hash` helper has been removed (Option E). Code that
/// previously called `H::concat_then_hash` should manually concatenate or
/// reinterpret raw memory into a contiguous byte slice before invoking `hash`.
pub trait HashT {
    /// Concrete hash output type.
    ///
    /// Must be:
    /// - `Copy` for efficient duplication
    /// - `Default` for representing an empty/default hash
    /// - `From<u8>` for domain separation prefixes
    /// - `Send + Sync` for use in concurrent contexts
    /// - `PartialEq + Debug + Hash` for comparisons, logging, and map/set usage
    type Output: core::hash::Hash
        + Default
        + Copy
        + PartialEq
        + core::fmt::Debug
        + From<u8>
        + Send
        + Sync;

    /// Hash arbitrary bytes into `Self::Output`.
    ///
    /// This is the only required operation. Higher-level helpers should be
    /// implemented externally if needed (e.g. concatenating multiple outputs
    /// or hashing structured data).
    fn hash(input: &[u8]) -> Self::Output;
}
