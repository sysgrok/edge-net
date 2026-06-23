//! An implementation of `edge-nal`'s TCP factory traits (`TcpAccept` / `TcpConnect`)
//! and socket traits (`Read` / `Write` / `Readable` / `TcpSplit` / `TcpShutdown`) over
//! TLS, by layering `mbedtls-rs` on top of an existing `edge-nal` TCP transport.
#![no_std]
#![allow(async_fn_in_trait)]
#![allow(clippy::uninlined_format_args)]

use core::net::SocketAddr;

use embedded_io_async::{ErrorType, Read, Write};

use edge_nal::{Close, Readable, TcpAccept, TcpConnect, TcpShutdown, TcpSplit};

use mbedtls_rs::{
    ClientSessionConfig, ServerSessionConfig, Session, SessionConfig, SessionError, Split,
    TlsReference,
};

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

/// Re-export of the `mbedtls-rs` crate, so that users don't have to explicitly depend on it
/// to e.g. create a `Tls` instance or build the session configurations.
pub mod mbedtls {
    pub use mbedtls_rs::*;
}

/// An implementation of `edge-nal`'s `TcpAccept` trait over TLS.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TlsAcceptor<'d, T> {
    acceptor: T,
    config: ServerSessionConfig<'d>,
    tls: TlsReference<'d>,
}

impl<'d, T> TlsAcceptor<'d, T>
where
    T: TcpAccept,
{
    /// Create a new instance of the `TlsAcceptor` type.
    ///
    /// # Arguments:
    /// - `tls` - A reference to the active `Tls` instance
    /// - `acceptor` - The underlying TCP acceptor
    /// - `config` - The server session configuration
    pub fn new(tls: TlsReference<'d>, acceptor: T, config: &ServerSessionConfig<'d>) -> Self {
        Self {
            acceptor,
            config: config.clone(),
            tls,
        }
    }
}

impl<T> TcpAccept for TlsAcceptor<'_, T>
where
    T: TcpAccept,
{
    type Error = SessionError;
    type Socket<'a>
        = TlsSocket<'a, T::Socket<'a>>
    where
        Self: 'a;

    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), <Self as TcpAccept>::Error> {
        let (addr, socket) = self
            .acceptor
            .accept()
            .await
            .map_err(SessionError::from_io)?;
        debug!("Accepted new connection on socket");

        let session = Session::new(
            self.tls,
            FromTcpSplit::new(socket),
            &SessionConfig::Server(self.config.clone()),
        )?;

        Ok((addr, TlsSocket(session)))
    }
}

/// An implementation of `edge-nal`'s `TcpConnect` trait over TLS.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TlsConnector<'d, T> {
    tls: TlsReference<'d>,
    connector: T,
    config: ClientSessionConfig<'d>,
}

impl<'d, T> TlsConnector<'d, T>
where
    T: TcpConnect,
{
    /// Create a new instance of the `TlsConnector` type.
    ///
    /// # Arguments:
    /// - `tls` - A reference to the active `Tls` instance
    /// - `connector` - The underlying TCP connector
    /// - `config` - The client session configuration
    pub fn new(tls: TlsReference<'d>, connector: T, config: &ClientSessionConfig<'d>) -> Self {
        Self {
            connector,
            config: config.clone(),
            tls,
        }
    }
}

impl<T> TcpConnect for TlsConnector<'_, T>
where
    T: TcpConnect,
{
    type Error = SessionError;

    type Socket<'a>
        = TlsSocket<'a, T::Socket<'a>>
    where
        Self: 'a;

    async fn connect(&self, remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        let socket = self
            .connector
            .connect(remote)
            .await
            .map_err(SessionError::from_io)?;
        debug!("Connected to {}", remote);

        let session = Session::new(
            self.tls,
            FromTcpSplit::new(socket),
            &SessionConfig::Client(self.config.clone()),
        )?;

        Ok(TlsSocket(session))
    }
}

/// A TLS socket, layered on top of an `edge-nal` TCP socket `T`.
///
/// This is the socket type returned by [`TlsAcceptor`] and [`TlsConnector`]. It implements
/// `edge-nal`'s `Read`, `Write`, `Readable`, `TcpSplit` and `TcpShutdown` traits and so can be
/// used anywhere an `edge-nal` TCP socket is expected (e.g. with `edge-http`).
///
/// It is a thin newtype wrapper around `mbedtls-rs`'s `Session`. The wrapper is needed - rather
/// than implementing the `edge-nal` traits directly on `Session` - because both the `edge-nal`
/// traits and `Session` are foreign to this crate, which the orphan rule does not allow.
pub struct TlsSocket<'d, T>(Session<'d, FromTcpSplit<T>>)
where
    T: Read + Write;

impl<'d, T> TlsSocket<'d, T>
where
    T: Read + Write,
{
    /// Get a shared reference to the underlying `mbedtls-rs` `Session`.
    pub fn session(&self) -> &Session<'d, FromTcpSplit<T>> {
        &self.0
    }

    /// Get a mutable reference to the underlying `mbedtls-rs` `Session`.
    pub fn session_mut(&mut self) -> &mut Session<'d, FromTcpSplit<T>> {
        &mut self.0
    }
}

impl<T> ErrorType for TlsSocket<'_, T>
where
    T: Read + Write,
{
    type Error = SessionError;
}

impl<T> Read for TlsSocket<'_, T>
where
    T: Read + Write,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<T> Write for TlsSocket<'_, T>
where
    T: Read + Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

impl<T> Readable for TlsSocket<'_, T>
where
    T: Read + Write + Readable,
{
    async fn readable(&mut self) -> Result<(), Self::Error> {
        // ... 1- because it is difficult to figure out - with the MbedTLS API - if `Session::read` would return without blocking
        // For this, we need support for that in MbedTLS itself, which is not available at the moment.
        // 2- because `Readable` currently throws exception with `edge-nal-embassy`
        Ok(())
    }
}

impl<T> TcpSplit for TlsSocket<'_, T>
where
    T: Read + Write + Readable + TcpSplit,
{
    type Read<'a>
        = Self
    where
        Self: 'a;

    type Write<'a>
        = Self
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        panic!("Splitting a TLS session is not supported yet");
    }
}

impl<T> TcpShutdown for TlsSocket<'_, T>
where
    T: Read + Write + TcpShutdown,
{
    async fn close(&mut self, what: Close) -> Result<(), Self::Error> {
        Session::close(&mut self.0).await?;

        self.0
            .stream()
            .close(what)
            .await
            .map_err(SessionError::from_io)
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        self.0.stream().abort().await.map_err(SessionError::from_io)
    }
}

/// An adaptor from `TcpSplit` to `Split` for types that implement `TcpSplit`.
///
/// Necessary so that `Session::split` works for streams implementing `edge-nal`'s `TcpSplit`.
pub struct FromTcpSplit<T>(T);

impl<T> FromTcpSplit<T> {
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }
}

impl<T> ErrorType for FromTcpSplit<T>
where
    T: ErrorType,
{
    type Error = T::Error;
}

impl<T> Split for FromTcpSplit<T>
where
    T: TcpSplit,
{
    type Read<'a>
        = T::Read<'a>
    where
        Self: 'a;

    type Write<'a>
        = T::Write<'a>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        self.0.split()
    }
}

impl<T> Read for FromTcpSplit<T>
where
    T: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<T> Write for FromTcpSplit<T>
where
    T: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

impl<T> Readable for FromTcpSplit<T>
where
    T: Readable,
{
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.readable().await
    }
}

impl<T> TcpSplit for FromTcpSplit<T>
where
    T: TcpSplit,
{
    type Read<'a>
        = T::Read<'a>
    where
        Self: 'a;

    type Write<'a>
        = T::Write<'a>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        self.0.split()
    }
}

impl<T> TcpShutdown for FromTcpSplit<T>
where
    T: TcpShutdown,
{
    async fn close(&mut self, what: Close) -> Result<(), Self::Error> {
        self.0.close(what).await
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        self.0.abort().await
    }
}
