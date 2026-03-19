#[macro_export]
macro_rules! nonzero {
    ($n:expr) => {
        std::num::NonZero::new($n).unwrap()
    };
}
