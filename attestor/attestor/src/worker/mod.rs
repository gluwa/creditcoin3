//! An API for distributing attestation tasks across threads for performance and DOS mitigation
//! reasons.
//!
//! # Overview
//!
//! The [`Worker`] api provides a common interface for the [production worker], [p2p worker] and
//! [validation worker] to make progress in the production, dissemination, validation and submission
//! of new attestations.
//!
//! New workers are registered via a [`CancellationMonitor`] and are spawned in separate, isolated
//! threads. The cancellation monitor is responsible for coordinating the progress of each worker by
//! driving them to completion and handling any potential failures. The monitor is meant to run from
//! the main thread, and should blocks until program completion.
//!
//! # Creating a new worker
//!
//! Worker threads must implement the [`Worker`] api, which provides a single entrypoint to drive a
//! worker to completion via the [`task`] method.
//!
//! ```
//! # use attestor::worker::Worker;
//! # use attestor::prelude::*;
//! #
//! struct MyCustomWorker;
//!
//! impl Worker for MyCustomWorker {
//!     type Error = std::io::Error;
//!
//!     async fn task(
//!         self,
//!         mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>
//!     ) -> attestor::worker::Exit<std::io::Error> {
//!         loop {
//!             tokio::select! {
//!                 _ = &mut shutdown => {
//!                     break Err(Interrupt::Stop);
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # Registering a worker
//!
//! Once created, a worker thread must be spawned via a [`CancellationMonitor`]. This returns a
//! [`JoinHandle`] which exits with the worker thread. It is the responsibility of the caller to
//! wait on each worker thread this way.
//!
//! ```
//! # use attestor::worker::CancellationMonitor;
//! # use attestor::worker::Worker;
//! # use attestor::prelude::*;
//! #
//! # struct MyCustomWorker;
//! #
//! # impl Worker for MyCustomWorker {
//! #   type Error = std::io::Error;
//! #
//! #   async fn task(
//! #       self,
//! #       shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>
//! #   ) -> attestor::worker::Exit<Self::Error> {
//! #       Ok(())
//! #   }
//! # }
//! #
//! # fn main() {
//! #
//! // Spawns a new worker thread
//! let monitor = CancellationMonitor::new();
//! let handle = monitor.spawn(MyCustomWorker);
//!
//! // Drives the thread to completion
//! handle.join().unwrap();
//! #
//! # }
//! ```
//!
//! [production worker]: production
//! [p2p worker]: p2p
//! [validation worker]: validation
//! [`task`]: Worker::task
//! [`JoinHandle`]: std::thread::JoinHandle

pub mod api;
pub mod p2p;
pub mod production;
pub mod validation;

use crate::prelude::*;

pub type Exit<E> = Result<(), Interrupt<E>>;

/// An API for spawning attestation tasks in their own dedicated thread.
pub trait Worker {
    type Error: std::error::Error + Send + Sync + 'static;

    /// The main task of a worker thread. Workers must not exit this method unless an unrecoverable
    /// error has occurred or the `shutdown` future has completed.
    ///
    /// ```rust
    /// # use attestor::worker::Worker;
    /// # use attestor::prelude::*;
    /// #
    /// # struct MyCustomWorker;
    /// #
    /// # impl Worker for MyCustomWorker {
    /// #   type Error = std::io::Error;
    /// async fn task(
    ///     self,
    ///     mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>
    /// ) -> attestor::worker::Exit<Self::Error> {
    ///     loop {
    ///         tokio::select! {
    ///             _ = &mut shutdown => {
    ///                 break Ok(());
    ///             }
    ///         }
    ///     }
    /// }
    /// # }
    /// ```
    fn task(
        self,
        shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = Exit<Self::Error>>;
}

/// A global thread monitor responsible for spawning and driving other [`Worker`] threads to
/// completion. The monitor is meant to run from the main thread, and should blocks until program
/// completion.
///
/// ```rust
/// # use attestor::worker::CancellationMonitor;
/// # use attestor::worker::Worker;
/// # use attestor::prelude::*;
/// #
/// # struct MyCustomWorker;
/// #
/// # impl Worker for MyCustomWorker {
/// #   type Error = std::io::Error;
/// #
/// #   async fn task(
/// #       self,
/// #       shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>
/// #   ) -> attestor::worker::Exit<Self::Error> {
/// #       Ok(())
/// #   }
/// # }
/// #
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// #
/// // Spawns a new worker thread
/// let monitor = CancellationMonitor::new();
/// let handle = monitor.spawn(MyCustomWorker);
///
/// // Wait for user-initiated shutdown
/// tokio::select! {
/// #   _ = std::future::ready(()) => {},
///     _ = tokio::signal::ctrl_c() => {
///         monitor.shutdown();
///     },
///     _ = monitor.cancelled() => {}
/// }
///
/// // Drives the thread to completion
/// handle.join().unwrap();
/// #
/// # }
/// ```
pub struct CancellationMonitor {
    shutdown: std::sync::Arc<tokio::sync::Notify>,
    failure: std::sync::Arc<tokio::sync::Notify>,
}

impl Default for CancellationMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationMonitor {
    /// Creates a new [`CancellationMonitor`].
    pub fn new() -> Self {
        Self {
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
            failure: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// A [`Future`] which resolves if the [`CancellationMonitor`] has been externally shutdown.
    ///
    /// [`Future`]: std::future::Future
    pub async fn cancelled(&self) {
        self.shutdown.notified().await
    }

    pub async fn failed(&self) {
        self.failure.notified().await
    }

    /// Shuts down the [`CancellationMonitor`] and notifies all [`Worker`] threads for a graceful
    /// shutdown.
    pub fn shutdown(self) {
        self.shutdown.notify_waiters();
    }

    /// Spawns a [`Worker`] into a new isolated thread, returning a [`JoinHandle`] to it. It is the
    /// responsibility of the caller to wait on each worker by consuming this handle.
    ///
    /// [`JoinHandle`]: std::thread::JoinHandle
    pub fn spawn<W: Worker + Send + 'static>(
        &self,
        worker: W,
    ) -> std::thread::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> {
        let shutdown = std::sync::Arc::clone(&self.shutdown);
        let failure = std::sync::Arc::clone(&self.failure);

        std::thread::spawn(move || {
            // TODO: properly handle this error
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to construct worker runtime");

            rt.block_on(async move {
                // From the tokio docs on `Notify::notified`
                //
                // >
                // > This method uses a queue to fairly distribute notifications in the order they
                // > were requested. Cancelling a call to notified makes you lose your place in the
                // > queue.
                // >
                //
                // To avoid this, we pin the `Notified` future to a stable address in memory and
                // keep re-using it across `select`s.
                let res = match worker.task(Box::pin(shutdown.notified())).await {
                    Ok(_) | Err(Interrupt::Stop) => Ok(()),
                    Err(Interrupt::Cont(err)) => Err(Box::from(err)),
                };

                // Notify error
                if res.is_err() {
                    failure.notify_waiters();
                }

                res
            })
        })
    }
}
