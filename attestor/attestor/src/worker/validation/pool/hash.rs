/// Based off [IdentityHasher]
///
/// [IdentityHasher]: https://docs.rs/identity-hash/latest/src/identity_hash/lib.rs.html#114

#[derive(Default)]
pub(crate) struct IdentityHasherU64(u64);

impl std::hash::Hasher for IdentityHasherU64 {
    fn write(&mut self, _bytes: &[u8]) {
        panic!("Invalid use of U64IdentityHasher")
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod test {
    use std::hash::Hasher as _;

    use super::*;

    #[test]
    fn hash_u64() {
        let mut hasher = IdentityHasherU64::default();
        hasher.write_u64(42);
        assert_eq!(hasher.finish(), 42);
    }

    #[test]
    fn hash_set() {
        let hasher = std::hash::BuildHasherDefault::<IdentityHasherU64>::default();
        let mut set = std::collections::HashSet::with_hasher(hasher);

        set.insert(42u64);
        assert!(set.contains(&42));
    }

    #[test]
    fn hash_map() {
        let hasher = std::hash::BuildHasherDefault::<IdentityHasherU64>::default();
        let mut set = std::collections::HashMap::with_hasher(hasher);

        set.insert(42u64, "Deep Thought");
        assert_eq!(set.get(&42), Some(&"Deep Thought"));
    }
}
