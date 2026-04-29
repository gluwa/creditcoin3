#[macro_export]
macro_rules! nonzero {
    ($n:expr) => {
        std::num::NonZero::new($n).unwrap()
    };
}

#[macro_export]
macro_rules! poll {
    ($stream:expr) => {{
        use futures::StreamExt as _;
        tokio_test::task::spawn($stream.next()).poll()
    }};
}
