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
}
