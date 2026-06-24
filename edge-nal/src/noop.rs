//! A no-op implementation of all `edge-nal` traits that panics when used.

use core::convert::Infallible;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use embedded_io_async::{ErrorType, Read, Write};

use crate::{
    AddrType, Close, Dns, MulticastV4, MulticastV6, Readable, TcpAccept, TcpBind, TcpConnect,
    TcpShutdown, TcpSplit, UdpBind, UdpConnect, UdpReceive, UdpSend, UdpSplit, UdpSplitMulticast,
};

/// A type that implements all `edge-nal` traits but does not support any operation
/// and panics when any method is called.
pub struct NoopNet;

impl UdpBind for NoopNet {
    type Error = Infallible;

    type Socket<'a>
        = NoopNet
    where
        Self: 'a;

    async fn bind(&self, _local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        panic!("UDP bind not supported")
    }
}

impl UdpConnect for NoopNet {
    type Error = Infallible;

    type Socket<'a>
        = NoopNet
    where
        Self: 'a;

    async fn connect(
        &self,
        _local: SocketAddr,
        _remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error> {
        panic!("UDP connect not supported")
    }
}

impl TcpBind for NoopNet {
    type Error = Infallible;

    type Accept<'a>
        = NoopNet
    where
        Self: 'a;

    async fn bind(&self, _local: SocketAddr) -> Result<Self::Accept<'_>, Self::Error> {
        panic!("TCP bind not supported")
    }
}

impl TcpAccept for NoopNet {
    type Error = Infallible;

    type Socket<'a>
        = NoopNet
    where
        Self: 'a;

    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        panic!("TCP accept not supported")
    }
}

impl TcpConnect for NoopNet {
    type Error = Infallible;

    type Socket<'a>
        = NoopNet
    where
        Self: 'a;

    async fn connect(&self, _remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        panic!("TCP connect not supported")
    }
}

impl Dns for NoopNet {
    type Error = Infallible;

    async fn get_host_by_name(
        &self,
        _host: &str,
        _addr_type: AddrType,
    ) -> Result<IpAddr, Self::Error> {
        panic!("DNS get_host_by_name not supported")
    }

    async fn get_host_by_address(
        &self,
        _addr: IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        panic!("DNS get_host_by_address not supported")
    }
}

impl ErrorType for NoopNet {
    type Error = Infallible;
}

impl Readable for NoopNet {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        panic!("Readable not supported")
    }
}

impl UdpSend for NoopNet {
    async fn send(&mut self, _remote: SocketAddr, _data: &[u8]) -> Result<(), Self::Error> {
        panic!("UDP send not supported")
    }
}

impl UdpReceive for NoopNet {
    async fn receive(&mut self, _buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        panic!("UDP receive not supported")
    }
}

impl UdpSplit for NoopNet {
    type Receive<'a> = Self;
    type Send<'a> = Self;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
        panic!("UDP split not supported")
    }
}

impl UdpSplitMulticast for NoopNet {
    type MulticastV4<'a> = Self;
    type MulticastV6<'a> = Self;

    fn split_multicast(
        &mut self,
    ) -> (
        Self::Receive<'_>,
        Self::Send<'_>,
        Self::MulticastV4<'_>,
        Self::MulticastV6<'_>,
    ) {
        panic!("UDP split multicast not supported")
    }
}

impl MulticastV4 for NoopNet {
    async fn join_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        panic!("Multicast join not supported")
    }

    async fn leave_v4(
        &mut self,
        _multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        panic!("Multicast leave not supported")
    }
}

impl MulticastV6 for NoopNet {
    async fn join_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        panic!("Multicast join not supported")
    }

    async fn leave_v6(
        &mut self,
        _multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        panic!("Multicast leave not supported")
    }
}

impl Read for NoopNet {
    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        panic!("Read not supported")
    }
}

impl Write for NoopNet {
    async fn write(&mut self, _buf: &[u8]) -> Result<usize, Self::Error> {
        panic!("Write not supported")
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        panic!("Flush not supported")
    }
}

impl TcpSplit for NoopNet {
    type Read<'a> = Self;
    type Write<'a> = Self;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        panic!("TCP split not supported")
    }
}

impl TcpShutdown for NoopNet {
    async fn close(&mut self, _what: Close) -> Result<(), Self::Error> {
        panic!("TCP shutdown not supported")
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        panic!("TCP abort not supported")
    }
}
