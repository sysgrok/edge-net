//! An implementation of `edge-nal`'s UDP factory traits (`UdpBind` / `UdpConnect`) and socket
//! traits (`UdpSend` / `UdpReceive` / `Readable` / `MulticastV4` / `MulticastV6` / `UdpSplit`)
//! over OpenThread (Thread), by wrapping the `openthread` crate's `OpenThread` handle and its
//! `UdpSocket`.
#![no_std]
#![allow(async_fn_in_trait)]
#![allow(clippy::uninlined_format_args)]

use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

use openthread::sys::otError_OT_ERROR_FAILED;
use openthread::{OpenThread, OtError, UdpSocket};

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

/// An `edge-nal` error wrapping `openthread`'s `OtError`.
///
/// This newtype is needed because the orphan rule forbids implementing `edge-nal`'s (foreign)
/// `io::Error` trait directly on `openthread`'s (foreign) `OtError`.
#[derive(Debug)]
pub struct OtNalError(OtError);

impl OtNalError {
    /// Get a reference to the underlying `openthread` `OtError`.
    pub fn inner(&self) -> &OtError {
        &self.0
    }
}

impl From<OtError> for OtNalError {
    fn from(err: OtError) -> Self {
        Self(err)
    }
}

impl core::fmt::Display for OtNalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

impl core::error::Error for OtNalError {}

impl edge_nal::io::Error for OtNalError {
    fn kind(&self) -> edge_nal::io::ErrorKind {
        // TODO
        edge_nal::io::ErrorKind::Other
    }
}

/// An `edge-nal` UDP stack, backed by an `openthread` `OpenThread` instance.
///
/// Implements `edge-nal`'s `UdpBind` and `UdpConnect` factory traits, producing [`OtUdpSocket`]s.
pub struct OtUdpStack<'a>(OpenThread<'a>);

impl<'a> OtUdpStack<'a> {
    /// Create a new UDP stack over the given `OpenThread` instance.
    pub const fn new(ot: OpenThread<'a>) -> Self {
        Self(ot)
    }

    /// Get a reference to the underlying `OpenThread` instance.
    pub fn openthread(&self) -> &OpenThread<'a> {
        &self.0
    }
}

impl<'a> edge_nal::UdpConnect for OtUdpStack<'a> {
    type Error = OtNalError;

    type Socket<'t>
        = OtUdpSocket<'a>
    where
        Self: 't;

    async fn connect(
        &self,
        _local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error> {
        // TODO: Local
        Ok(OtUdpSocket(UdpSocket::connect(
            self.0.clone(),
            &socket_addr_v6(remote)?,
        )?))
    }
}

impl<'a> edge_nal::UdpBind for OtUdpStack<'a> {
    type Error = OtNalError;

    type Socket<'t>
        = OtUdpSocket<'a>
    where
        Self: 't;

    async fn bind(&self, addr: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        Ok(OtUdpSocket(UdpSocket::bind(
            self.0.clone(),
            &socket_addr_v6(addr)?,
        )?))
    }
}

/// An `edge-nal` UDP socket, backed by an `openthread` `UdpSocket`.
pub struct OtUdpSocket<'a>(UdpSocket<'a>);

impl<'a> OtUdpSocket<'a> {
    /// Get a reference to the underlying `openthread` `UdpSocket`.
    pub fn inner(&self) -> &UdpSocket<'a> {
        &self.0
    }
}

impl edge_nal::io::ErrorType for OtUdpSocket<'_> {
    type Error = OtNalError;
}

impl edge_nal::UdpSend for OtUdpSocket<'_> {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        self.0.send(data, None, &socket_addr_v6(remote)?).await?;

        Ok(())
    }
}

impl edge_nal::UdpReceive for OtUdpSocket<'_> {
    async fn receive(&mut self, buf: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let (len, _, addr) = self.0.recv(buf).await?;

        Ok((len, addr.into()))
    }
}

impl edge_nal::Readable for OtUdpSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.wait_recv_available().await?;

        Ok(())
    }
}

impl edge_nal::MulticastV4 for OtUdpSocket<'_> {
    async fn join_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }

    async fn leave_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }
}

impl edge_nal::MulticastV6 for OtUdpSocket<'_> {
    async fn join_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }

    async fn leave_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }
}

impl edge_nal::UdpSocket for OtUdpSocket<'_> {}

impl edge_nal::UdpSplit for OtUdpSocket<'_> {
    type Receive<'a>
        = &'a Self
    where
        Self: 'a;

    type Send<'a>
        = &'a Self
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
        (&*self, &*self)
    }
}

impl edge_nal::io::ErrorType for &OtUdpSocket<'_> {
    type Error = OtNalError;
}

impl edge_nal::UdpSend for &OtUdpSocket<'_> {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        self.0.send(data, None, &socket_addr_v6(remote)?).await?;

        Ok(())
    }
}

impl edge_nal::UdpReceive for &OtUdpSocket<'_> {
    async fn receive(&mut self, buf: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let (len, _, addr) = self.0.recv(buf).await?;

        Ok((len, addr.into()))
    }
}

impl edge_nal::Readable for &OtUdpSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.wait_recv_available().await?;

        Ok(())
    }
}

impl edge_nal::MulticastV4 for &OtUdpSocket<'_> {
    async fn join_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }

    async fn leave_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }
}

impl edge_nal::MulticastV6 for &OtUdpSocket<'_> {
    async fn join_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }

    async fn leave_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        warn!("Multicast not supported with Thread networks");

        Ok(())
    }
}

impl edge_nal::UdpSocket for &OtUdpSocket<'_> {}

fn socket_addr_v6(addr: SocketAddr) -> Result<SocketAddrV6, OtNalError> {
    match addr {
        SocketAddr::V4(_) => Err(OtNalError(OtError::new(otError_OT_ERROR_FAILED))),
        SocketAddr::V6(v6) => Ok(v6),
    }
}
