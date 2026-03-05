//! Type-safe primitives for propagating user-initiated cancellation requests. Any method which
//! returns an [`Interrupt`] indicates a possible user cancellation.
//!
//! # Potential improvements
//!
//! Ideally we would use something like:
//!
//! ```rust
//! enum Interrupt<T, E> {
//!     Ok(T),
//!     Err(E),
//!     Stop
//! }
//! ```
//!
//! However, since the [`Try`] trait is not yet stable in Rust we need to be a bit more creative if
//! we want to maintain a good DevEx.
//!
//! [`Try`]: https://doc.rust-lang.org/stable/std/ops/trait.Try.html

pub mod prelude {
    pub use crate::Interrupt;
    pub use crate::MapInterrupt;
    pub use crate::OkInterrupt;
}

#[derive(Debug)]
pub enum Interrupt<T> {
    Cont(T),
    Stop,
}

impl<D: std::fmt::Display> std::fmt::Display for Interrupt<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cont(err) => write!(f, "{err}"),
            Self::Stop => Ok(()),
        }
    }
}

impl<E: std::error::Error> std::error::Error for Interrupt<E> {}

pub trait MapInterrupt<T, E, F> {
    fn map_interrupt(self, op: impl FnOnce(E) -> F) -> Result<T, Interrupt<F>>;
}

impl<T, E, F> MapInterrupt<T, E, F> for Result<T, Interrupt<E>> {
    fn map_interrupt(self, op: impl FnOnce(E) -> F) -> Result<T, Interrupt<F>> {
        self.map_err(|interrupt| match interrupt {
            Interrupt::Cont(err) => Interrupt::Cont(op(err)),
            Interrupt::Stop => Interrupt::Stop,
        })
    }
}

impl<T, E, F> MapInterrupt<T, E, F> for Result<T, E> {
    fn map_interrupt(self, op: impl FnOnce(E) -> F) -> Result<T, Interrupt<F>> {
        match self {
            Ok(res) => Ok(res),
            Err(err) => Err(Interrupt::Cont(op(err))),
        }
    }
}

pub trait OkInterrupt<T, E> {
    fn ok_interrupt(self, err: E) -> Result<T, Interrupt<E>>;
}

impl<T, E> OkInterrupt<T, E> for Option<T> {
    fn ok_interrupt(self, err: E) -> Result<T, Interrupt<E>> {
        self.ok_or(Interrupt::Cont(err))
    }
}
