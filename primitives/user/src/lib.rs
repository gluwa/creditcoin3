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
    pub use crate::Shutdown;
    pub use crate::ShutdownInterrupt;
    pub use crate::UnwrapInterrupt;
}

/// Typed marker error representing a user-initiated graceful shutdown (e.g. Ctrl+C /
/// service stop) that propagated out of an [`Interrupt::Stop`].
///
/// Production code paths convert [`Interrupt::Stop`] into this error (via
/// [`ShutdownInterrupt::propagate_shutdown`]) instead of panicking. Because it implements
/// [`std::error::Error`], it can be carried inside an `anyhow::Error` and recognized by outer
/// layers (which downcast for it) so that a shutdown results in a clean exit rather than a
/// reconnect attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shutdown;

impl std::fmt::Display for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "shutdown requested")
    }
}

impl std::error::Error for Shutdown {}

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

pub trait UnwrapInterrupt<T, E> {
    fn unwrap_interrupt(self, msg: &'static str) -> Result<T, E>;
}

impl<T, E> UnwrapInterrupt<T, E> for Result<T, Interrupt<E>> {
    fn unwrap_interrupt(self, msg: &'static str) -> Result<T, E> {
        match self {
            Ok(ok) => Ok(ok),
            Err(Interrupt::Cont(err)) => Err(err),
            Err(Interrupt::Stop) => panic!("{msg}"),
        }
    }
}

/// Propagate an [`Interrupt`] as an ordinary error **without panicking on [`Interrupt::Stop`]**.
///
/// Unlike [`UnwrapInterrupt::unwrap_interrupt`] (which panics on `Stop`, and is therefore only
/// safe in code that never receives a user interrupt), this converts:
/// - [`Interrupt::Cont(err)`] into the underlying error `err`, and
/// - [`Interrupt::Stop`] into the typed [`Shutdown`] marker error,
///
/// both projected into the caller's chosen error type `F` via [`From`]. With an `anyhow::Error`
/// target this lets shutdown flow out of production block-fetch paths as a normal `Result::Err`
/// that outer layers can recognize (by downcasting to [`Shutdown`]) and treat as a clean exit.
pub trait ShutdownInterrupt<T, E> {
    fn propagate_shutdown<F>(self) -> Result<T, F>
    where
        F: From<E> + From<Shutdown>;
}

impl<T, E> ShutdownInterrupt<T, E> for Result<T, Interrupt<E>> {
    fn propagate_shutdown<F>(self) -> Result<T, F>
    where
        F: From<E> + From<Shutdown>,
    {
        match self {
            Ok(ok) => Ok(ok),
            Err(Interrupt::Cont(err)) => Err(F::from(err)),
            Err(Interrupt::Stop) => Err(F::from(Shutdown)),
        }
    }
}
