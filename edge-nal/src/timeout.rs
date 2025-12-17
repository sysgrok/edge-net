//! This module provides utility function and a decorator struct
//! for adding timeouts to IO types.
//!
//! Note that the presence of this module in the `edge-nal` crate
//! is a bit controversial, as it is a utility, while `edge-nal` is a
//! pure traits' crate otherwise.
//!
//! Therefore, the module might be moved to another location in future.

use core::{
    fmt::{self, Display},
    future::Future,
    net::SocketAddr,
};

use embassy_time::Duration;
use embedded_io_async::{ErrorKind, ErrorType, Read, Write};

use crate::{Readable, TcpAccept, TcpConnect, TcpShutdown, TcpSplit};

/// Error type for the `with_timeout` function and `WithTimeout` struct.
#[derive(Debug)]
pub enum WithTimeoutError<E> {
    /// An error occurred during the execution of the operation
    Error(E),
    /// The operation timed out
    Timeout,
}

impl<E> From<E> for WithTimeoutError<E> {
    fn from(e: E) -> Self {
        Self::Error(e)
    }
}

impl<E> fmt::Display for WithTimeoutError<E>
where
    E: Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error(e) => write!(f, "{}", e),
            Self::Timeout => write!(f, "Operation timed out"),
        }
    }
}

impl<E: embedded_io_async::Error> core::error::Error for WithTimeoutError<E> {}

impl<E> embedded_io_async::Error for WithTimeoutError<E>
where
    E: embedded_io_async::Error,
{
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Error(e) => e.kind(),
            Self::Timeout => ErrorKind::TimedOut,
        }
    }
}

/// Run a fallible future with a timeout.
///
/// A future is a fallible future if it resolves to a `Result<T, E>`.
///
/// If the future completes before the timeout, its output is returned.
/// Otherwise, on timeout, a timeout error is returned.
///
/// Parameters:
/// - `timeout_ms`: The timeout duration in milliseconds
/// - `fut`: The future to run
pub async fn with_timeout<F, T, E>(timeout_ms: u32, fut: F) -> Result<T, WithTimeoutError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    map_result(embassy_time::with_timeout(Duration::from_millis(timeout_ms as _), fut).await)
}

/// A type that wraps an IO stream type and adds a timeout to all operations.
///
/// The operations decorated with a timeout are the ones offered via the following traits:
/// - `embedded_io_async::Read`
/// - `embedded_io_async::Write`
/// - `Readable`
/// - `TcpConnect`
/// - `TcpShutdown`
///
/// Additionally, wrapping with `WithTimeout` an IO type that implements `TcpAccept` will result
/// in a `TcpAccept` implementation that - while waiting potentially indefinitely for an incoming
/// connection - will return a connected socket readily wrapped with a timeout.
pub struct WithTimeout<T>(T, u32);

impl<T> WithTimeout<T> {
    /// Create a new `WithTimeout` instance.
    ///
    /// Parameters:
    /// - `timeout_ms`: The timeout duration in milliseconds
    /// - `io`: The IO type to add a timeout to
    pub const fn new(timeout_ms: u32, io: T) -> Self {
        Self(io, timeout_ms)
    }

    /// Get a reference to the inner IO type.
    pub fn io(&self) -> &T {
        &self.0
    }

    /// Get a mutable reference to the inner IO type.
    pub fn io_mut(&mut self) -> &mut T {
        &mut self.0
    }

    /// Get the timeout duration in milliseconds.
    pub fn timeout_ms(&self) -> u32 {
        self.1
    }

    /// Get the IO type by destructuring the `WithTimeout` instance.
    pub fn into_io(self) -> T {
        self.0
    }
}

impl<T> ErrorType for WithTimeout<T>
where
    T: ErrorType,
{
    type Error = WithTimeoutError<T::Error>;
}

impl<T> Read for WithTimeout<T>
where
    T: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        with_timeout(self.1, self.0.read(buf)).await
    }
}

impl<T> Write for WithTimeout<T>
where
    T: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        with_timeout(self.1, self.0.write(buf)).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        with_timeout(self.1, self.0.flush()).await
    }
}

impl<T> TcpConnect for WithTimeout<T>
where
    T: TcpConnect,
{
    type Error = WithTimeoutError<T::Error>;

    type Socket<'a>
        = WithTimeout<T::Socket<'a>>
    where
        Self: 'a;

    async fn connect(&self, remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        with_timeout(self.1, self.0.connect(remote))
            .await
            .map(|s| WithTimeout::new(self.1, s))
    }
}

impl<T> Readable for WithTimeout<T>
where
    T: Readable,
{
    async fn readable(&mut self) -> Result<(), Self::Error> {
        with_timeout(self.1, self.0.readable()).await
    }
}

impl<T> TcpSplit for WithTimeout<T>
where
    T: TcpSplit,
{
    type Read<'a>
        = WithTimeout<T::Read<'a>>
    where
        Self: 'a;

    type Write<'a>
        = WithTimeout<T::Write<'a>>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        let (r, w) = self.0.split();
        (WithTimeout::new(self.1, r), WithTimeout::new(self.1, w))
    }
}

impl<T> TcpShutdown for WithTimeout<T>
where
    T: TcpShutdown,
{
    async fn close(&mut self, what: crate::Close) -> Result<(), Self::Error> {
        with_timeout(self.1, self.0.close(what)).await
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        with_timeout(self.1, self.0.abort()).await
    }
}

impl<T> TcpAccept for WithTimeout<T>
where
    T: TcpAccept,
{
    type Error = WithTimeoutError<T::Error>;

    type Socket<'a>
        = WithTimeout<T::Socket<'a>>
    where
        Self: 'a;

    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        let (addr, socket) = self.0.accept().await?;

        Ok((addr, WithTimeout::new(self.1, socket)))
    }
}

fn map_result<T, E>(
    result: Result<Result<T, E>, embassy_time::TimeoutError>,
) -> Result<T, WithTimeoutError<E>> {
    match result {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => Err(WithTimeoutError::Error(e)),
        Err(_) => Err(WithTimeoutError::Timeout),
    }
}
