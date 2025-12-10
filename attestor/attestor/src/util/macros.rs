#[macro_export]
macro_rules! ensure {
    ($cond:expr, $err:expr) => {
        if !($cond) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! hash_set {
    () => {
        std::collections::HashSet::new()
    };
    ($($item:expr),*) => {
        {
            let mut hash_set = std::collections::HashSet::new();
            $(
                hash_set.insert($item);
            )*
            hash_set
        }
    };
}

#[macro_export]
macro_rules! hash_map {
    () => {
        std::collections::HashMap::new()
    };
    ($($key:expr => $value:expr),*) => {
        {
            let mut hash_map = std::collections::HashMap::new();
            $(
                hash_map.insert($key, $value);
            )*
            hash_map
        }
    };
}

#[macro_export]
macro_rules! btree_map {
    () => {
        std::collections::BTreeMap::new()
    };
    ($($key:expr => $value:expr),*) => {
        {
            let mut btree_map = std::collections::BTreeMap::new();
            $(
                btree_map.insert($key, $value);
            )*
            btree_map
        }
    };
}

#[cfg(test)]
mod test {
    #[test]
    fn macro_ensure() {
        let err = || {
            ensure!(1 == 0, "1 != 0");
            Ok(())
        };
        assert_matches::assert_matches!(err(), Err("1 != 0"));
    }

    #[test]
    fn macro_hash_set() {
        let set = hash_set![1, 2, 3, 4, 5];

        let mut reference = std::collections::HashSet::new();
        reference.insert(1);
        reference.insert(2);
        reference.insert(3);
        reference.insert(4);
        reference.insert(5);

        assert_eq!(set, reference);
    }

    #[test]
    fn macro_hash_map() {
        let map = hash_map![1 => 'a', 2 => 'b', 3 => 'c'];

        let mut reference = std::collections::HashMap::new();
        reference.insert(1, 'a');
        reference.insert(2, 'b');
        reference.insert(3, 'c');

        assert_eq!(map, reference);
    }
}
