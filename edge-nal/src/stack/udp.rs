//! Factory traits for creating UDP sockets on embedded devices

use core::net::SocketAddr;

use embedded_io_async::ErrorType;

use crate::udp::{UdpReceive, UdpSend};
use crate::{MulticastV4, MulticastV6, Readable};

/// This trait is implemented by UDP sockets that can be split into separate `send` and `receive` halves that can operate
/// independently from each other (i.e., a full-duplex connection)
pub trait UdpSplit: ErrorType {
    type Receive<'a>: UdpReceive<Error = Self::Error> + Readable<Error = Self::Error>
    where
        Self: 'a;
    type Send<'a>: UdpSend<Error = Self::Error>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>);
}

impl<T> UdpSplit for &mut T
where
    T: UdpSplit,
{
    type Receive<'a>
        = T::Receive<'a>
    where
        Self: 'a;
    type Send<'a>
        = T::Send<'a>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
        (**self).split()
    }
}

/// This trait is implemented by UDP sockets that can be split into separate
/// `send`, `receive`, multicastv4, and multicastv6 parts that can operate independently from each other
/// (i.e., a full-duplex connection with multicasting capabilities)
pub trait UdpSplitMulticast: UdpSplit {
    type MulticastV4<'a>: MulticastV4<Error = Self::Error>
    where
        Self: 'a;
    type MulticastV6<'a>: MulticastV6<Error = Self::Error>
    where
        Self: 'a;

    fn split_multicast(
        &mut self,
    ) -> (
        Self::Receive<'_>,
        Self::Send<'_>,
        Self::MulticastV4<'_>,
        Self::MulticastV6<'_>,
    );
}

impl<T> UdpSplitMulticast for &mut T
where
    T: UdpSplitMulticast,
{
    type MulticastV4<'a>
        = T::MulticastV4<'a>
    where
        Self: 'a;
    type MulticastV6<'a>
        = T::MulticastV6<'a>
    where
        Self: 'a;

    fn split_multicast(
        &mut self,
    ) -> (
        Self::Receive<'_>,
        Self::Send<'_>,
        Self::MulticastV4<'_>,
        Self::MulticastV6<'_>,
    ) {
        (**self).split_multicast()
    }
}

/// This is a factory trait for creating connected UDP sockets
pub trait UdpConnect {
    /// Error type returned on socket creation failure
    type Error: embedded_io_async::Error;

    /// The socket type returned by the factory
    type Socket<'a>: UdpReceive<Error = Self::Error>
        + UdpSend<Error = Self::Error>
        + UdpSplitMulticast<Error = Self::Error>
        + MulticastV4<Error = Self::Error>
        + MulticastV6<Error = Self::Error>
        + Readable<Error = Self::Error>
    where
        Self: 'a;

    /// Connect to a remote socket
    async fn connect(
        &self,
        local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error>;
}

/// This is a factory trait for binding UDP sockets
pub trait UdpBind {
    /// Error type returned on socket creation failure
    type Error: embedded_io_async::Error;

    /// The socket type returned by the stack
    type Socket<'a>: UdpReceive<Error = Self::Error>
        + UdpSend<Error = Self::Error>
        + UdpSplitMulticast<Error = Self::Error>
        + MulticastV4<Error = Self::Error>
        + MulticastV6<Error = Self::Error>
        + Readable<Error = Self::Error>
    where
        Self: 'a;

    /// Bind to a local socket address
    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error>;
}

impl<T> UdpConnect for &T
where
    T: UdpConnect,
{
    type Error = T::Error;
    type Socket<'a>
        = T::Socket<'a>
    where
        Self: 'a;

    async fn connect(
        &self,
        local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error> {
        (*self).connect(local, remote).await
    }
}

impl<T> UdpConnect for &mut T
where
    T: UdpConnect,
{
    type Error = T::Error;
    type Socket<'a>
        = T::Socket<'a>
    where
        Self: 'a;

    async fn connect(
        &self,
        local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error> {
        (**self).connect(local, remote).await
    }
}

impl<T> UdpBind for &T
where
    T: UdpBind,
{
    type Error = T::Error;
    type Socket<'a>
        = T::Socket<'a>
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        (*self).bind(local).await
    }
}

impl<T> UdpBind for &mut T
where
    T: UdpBind,
{
    type Error = T::Error;
    type Socket<'a>
        = T::Socket<'a>
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        (**self).bind(local).await
    }
}
