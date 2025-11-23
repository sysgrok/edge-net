#![allow(async_fn_in_trait)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::ops::Deref;
use core::pin::pin;

use std::io;
use std::net::{self, Shutdown, TcpStream, ToSocketAddrs, UdpSocket as StdUdpSocket};

#[cfg(not(feature = "async-io-mini"))]
use async_io::Async;
#[cfg(feature = "async-io-mini")]
use async_io_mini::Async;

use futures_lite::io::{AsyncReadExt, AsyncWriteExt};

use embedded_io_async::{ErrorType, Read, Write};

use edge_nal::{
    AddrType, Dns, MulticastV4, MulticastV6, Readable, TcpAccept, TcpBind, TcpConnect, TcpShutdown,
    TcpSplit, UdpBind, UdpConnect, UdpReceive, UdpSend, UdpSplit,
};

#[cfg(any(target_os = "linux", target_os = "android"))]
pub use raw::*;

/// The STD network stack implementation.
///
/// This uses the standard library's networking types under the hood,
/// wrapped in async-io's `Async` type for async support.
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Default, Copy, Clone)]
pub struct Stack(());

impl Stack {
    /// Create a new STD network stack instance.
    pub const fn new() -> Self {
        Self(())
    }
}

impl TcpConnect for Stack {
    type Error = io::Error;

    type Socket<'a>
        = TcpSocket
    where
        Self: 'a;

    async fn connect(&self, remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        let socket = Async::<TcpStream>::connect(remote).await?;

        Ok(TcpSocket(socket))
    }
}

impl TcpBind for Stack {
    type Error = io::Error;

    type Accept<'a>
        = TcpAcceptor
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Accept<'_>, Self::Error> {
        let acceptor = Async::<net::TcpListener>::bind(local).map(TcpAcceptor)?;

        Ok(acceptor)
    }
}

/// The TCP acceptor type for the STD network stack.
pub struct TcpAcceptor(Async<net::TcpListener>);

impl TcpAccept for TcpAcceptor {
    type Error = io::Error;

    type Socket<'a>
        = TcpSocket
    where
        Self: 'a;

    #[cfg(not(target_os = "espidf"))]
    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        let socket = self.0.accept().await.map(|(socket, _)| socket)?;

        Ok((socket.as_ref().peer_addr()?, TcpSocket(socket)))
    }

    #[cfg(target_os = "espidf")]
    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        // ESP IDF (lwIP actually) does not really support `select`-ing on
        // socket accept: https://groups.google.com/g/osdeve_mirror_tcpip_lwip/c/Vsz7SVa6a2M
        //
        // If we do this, `select` would block and not return with our accepting socket `fd`
        // marked as ready even if our accepting socket has a pending connection.
        //
        // (Note also that since the time when the above link was posted on the internet,
        // the lwIP `accept` API has improved a bit in that it would now return `EWOULDBLOCK`
        // instead of blocking indefinitely
        // - and we take advantage of that in the "async" implementation below.)
        //
        // The workaround below is not ideal in that
        // it uses a timer to poll the socket, but it avoids spinning a hidden,
        // separate thread just to accept connections - which would be the alternative.
        loop {
            match self.0.as_ref().accept() {
                Ok((socket, _)) => break Ok((socket.peer_addr()?, TcpSocket(Async::new(socket)?))),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    #[cfg(not(feature = "async-io-mini"))]
                    use async_io::Timer;
                    #[cfg(feature = "async-io-mini")]
                    use async_io_mini::Timer;

                    Timer::after(core::time::Duration::from_millis(20)).await;
                }
                Err(err) => break Err(err),
            }
        }
    }
}

/// The TCP socket type for the STD network stack.
pub struct TcpSocket(Async<TcpStream>);

impl TcpSocket {
    /// Create a new TCP socket from the given async TCP stream.
    ///
    /// # Arguments
    /// - `socket`: The async TCP stream to wrap.
    pub const fn new(socket: Async<TcpStream>) -> Self {
        Self(socket)
    }

    /// Release the underlying async TCP stream.
    pub fn release(self) -> Async<TcpStream> {
        self.0
    }
}

impl Deref for TcpSocket {
    type Target = Async<TcpStream>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ErrorType for TcpSocket {
    type Error = io::Error;
}

impl Read for TcpSocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl Write for TcpSocket {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}

impl Readable for TcpSocket {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.readable().await
    }
}

impl ErrorType for &TcpSocket {
    type Error = io::Error;
}

impl Read for &TcpSocket {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        (&self.0).read(buf).await
    }
}

impl Write for &TcpSocket {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        (&self.0).write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        (&self.0).flush().await
    }
}

impl Readable for &TcpSocket {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.readable().await
    }
}

impl TcpSplit for TcpSocket {
    type Read<'a>
        = &'a TcpSocket
    where
        Self: 'a;

    type Write<'a>
        = &'a TcpSocket
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        let socket = &*self;

        (socket, socket)
    }
}

impl TcpShutdown for TcpSocket {
    async fn close(&mut self, what: edge_nal::Close) -> Result<(), Self::Error> {
        match what {
            edge_nal::Close::Read => self.0.as_ref().shutdown(Shutdown::Read)?,
            edge_nal::Close::Write => self.0.as_ref().shutdown(Shutdown::Write)?,
            edge_nal::Close::Both => self.0.as_ref().shutdown(Shutdown::Both)?,
        }

        Ok(())
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        // No-op, STD will abort the socket on drop anyway

        Ok(())
    }
}

impl UdpConnect for Stack {
    type Error = io::Error;

    type Socket<'a>
        = UdpSocket
    where
        Self: 'a;

    async fn connect(
        &self,
        local: SocketAddr,
        remote: SocketAddr,
    ) -> Result<Self::Socket<'_>, Self::Error> {
        let socket = Async::<StdUdpSocket>::bind(local)?;

        socket.as_ref().connect(remote)?;

        Ok(UdpSocket(socket))
    }
}

impl UdpBind for Stack {
    type Error = io::Error;

    type Socket<'a>
        = UdpSocket
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        let socket = Async::<StdUdpSocket>::bind(local)?;

        socket.as_ref().set_broadcast(true)?;

        Ok(UdpSocket(socket))
    }
}

/// The UDP socket type for the STD network stack.
pub struct UdpSocket(Async<StdUdpSocket>);

impl UdpSocket {
    /// Create a new UDP socket from the given async UDP socket.
    ///
    /// # Arguments
    /// - `socket`: The async UDP socket to wrap.
    pub const fn new(socket: Async<StdUdpSocket>) -> Self {
        Self(socket)
    }

    /// Release the underlying async UDP socket.
    pub fn release(self) -> Async<StdUdpSocket> {
        self.0
    }

    /// Join a multicast group for IPv4.
    ///
    /// # Arguments
    /// - `multiaddr`: The multicast address to join.
    /// - `interface`: The interface address to use.
    pub fn join_multicast_v4(
        &self,
        multiaddr: &Ipv4Addr,
        interface: &Ipv4Addr,
    ) -> Result<(), io::Error> {
        #[cfg(not(target_os = "espidf"))]
        self.as_ref().join_multicast_v4(multiaddr, interface)?;

        #[cfg(target_os = "espidf")]
        self.setsockopt_ipproto_ip(
            multiaddr, interface, 3, /* IP_ADD_MEMBERSHIP in ESP IDF*/
        )?;

        Ok(())
    }

    /// Leave a multicast group for IPv4.
    ///
    /// # Arguments
    /// - `multiaddr`: The multicast address to leave.
    /// - `interface`: The interface address to use.
    pub fn leave_multicast_v4(
        &self,
        multiaddr: &Ipv4Addr,
        interface: &Ipv4Addr,
    ) -> Result<(), io::Error> {
        #[cfg(not(target_os = "espidf"))]
        self.as_ref().leave_multicast_v4(multiaddr, interface)?;

        #[cfg(target_os = "espidf")]
        self.setsockopt_ipproto_ip(
            multiaddr, interface, 4, /* IP_LEAVE_MEMBERSHIP in ESP IDF*/
        )?;

        Ok(())
    }

    #[cfg(target_os = "espidf")]
    pub fn setsockopt_ipproto_ip(
        &self,
        multiaddr: &Ipv4Addr,
        interface: &Ipv4Addr,
        option: u32,
    ) -> Result<(), io::Error> {
        // join_multicast_v4() is broken for ESP-IDF due to IP_ADD_MEMBERSHIP being wrongly defined to 11,
        // while it should be 3: https://github.com/rust-lang/libc/blob/main/src/unix/newlib/mod.rs#L568
        //
        // leave_multicast_v4() is broken for ESP-IDF due to IP_ADD_MEMBERSHIP being wrongly defined to 12,
        // while it should be 4: https://github.com/rust-lang/libc/blob/main/src/unix/newlib/mod.rs#L569

        let mreq = sys::ip_mreq {
            imr_multiaddr: sys::in_addr {
                s_addr: u32::from_ne_bytes(multiaddr.octets()),
            },
            imr_interface: sys::in_addr {
                s_addr: u32::from_ne_bytes(interface.octets()),
            },
        };

        use std::os::fd::AsRawFd;

        syscall_los!(unsafe {
            sys::setsockopt(
                self.0.as_raw_fd(),
                sys::IPPROTO_IP as _,
                option as _,
                &mreq as *const _ as *const _,
                core::mem::size_of::<sys::ip_mreq>() as _,
            )
        })?;

        Ok(())
    }
}

impl Deref for UdpSocket {
    type Target = Async<StdUdpSocket>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ErrorType for &UdpSocket {
    type Error = io::Error;
}

impl UdpReceive for &UdpSocket {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let remote = self.0.as_ref().peer_addr();

        let (len, remote) = if let Ok(remote) = remote {
            // Connected socket
            let fut = pin!(self.0.recv(buffer));
            let len = fut.await?;

            (len, remote)
        } else {
            // Unconnected socket
            let fut = pin!(self.0.recv_from(buffer));
            let (len, remote) = fut.await?;

            (len, remote)
        };

        Ok((len, remote))
    }
}

impl UdpSend for &UdpSocket {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        let is_remote = self.0.as_ref().peer_addr().is_ok();

        if is_remote {
            // Connected socket
            let mut offset = 0;

            loop {
                let fut = pin!(self.0.send(&data[offset..]));
                offset += fut.await?;

                if offset == data.len() {
                    break;
                }
            }
        } else {
            // Unconnected socket
            let mut offset = 0;

            loop {
                let fut = pin!(self.0.send_to(&data[offset..], remote));
                offset += fut.await?;

                if offset == data.len() {
                    break;
                }
            }
        }

        Ok(())
    }
}

impl MulticastV4 for &UdpSocket {
    async fn join_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        self.join_multicast_v4(&multicast_addr, &interface)
    }

    async fn leave_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        self.leave_multicast_v4(&multicast_addr, &interface)
    }
}

impl MulticastV6 for &UdpSocket {
    async fn join_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        self.0
            .as_ref()
            .join_multicast_v6(&multicast_addr, interface)
    }

    async fn leave_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        self.0
            .as_ref()
            .leave_multicast_v6(&multicast_addr, interface)
    }
}

impl Readable for &UdpSocket {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.readable().await
    }
}

impl ErrorType for UdpSocket {
    type Error = io::Error;
}

impl UdpReceive for UdpSocket {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let mut rself = &*self;

        let fut = pin!(rself.receive(buffer));
        fut.await
    }
}

impl UdpSend for UdpSocket {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        let mut rself = &*self;

        let fut = pin!(rself.send(remote, data));
        fut.await
    }
}

impl MulticastV4 for UdpSocket {
    async fn join_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        self.join_multicast_v4(&multicast_addr, &interface)
    }

    async fn leave_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        self.leave_multicast_v4(&multicast_addr, &interface)
    }
}

impl MulticastV6 for UdpSocket {
    async fn join_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        self.0
            .as_ref()
            .join_multicast_v6(&multicast_addr, interface)
    }

    async fn leave_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        self.0
            .as_ref()
            .leave_multicast_v6(&multicast_addr, interface)
    }
}

impl Readable for UdpSocket {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        let mut rself = &*self;

        let fut = pin!(rself.readable());
        fut.await
    }
}

impl UdpSplit for UdpSocket {
    type Receive<'a>
        = &'a Self
    where
        Self: 'a;

    type Send<'a>
        = &'a Self
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
        let socket = &*self;

        (socket, socket)
    }
}

impl Dns for Stack {
    type Error = io::Error;

    async fn get_host_by_name(
        &self,
        host: &str,
        addr_type: AddrType,
    ) -> Result<IpAddr, Self::Error> {
        let host = host.to_string();

        dns_lookup_host(&host, addr_type)
    }

    async fn get_host_by_address(
        &self,
        _addr: IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Perform a DNS lookup for the given host and address type.
fn dns_lookup_host(host: &str, addr_type: AddrType) -> Result<IpAddr, io::Error> {
    (host, 0_u16)
        .to_socket_addrs()?
        .find(|addr| match addr_type {
            AddrType::IPv4 => matches!(addr, std::net::SocketAddr::V4(_)),
            AddrType::IPv6 => matches!(addr, std::net::SocketAddr::V6(_)),
            AddrType::Either => true,
        })
        .map(|addr| match addr {
            std::net::SocketAddr::V4(v4) => v4.ip().octets().into(),
            std::net::SocketAddr::V6(v6) => v6.ip().octets().into(),
        })
        .ok_or_else(|| io::ErrorKind::AddrNotAvailable.into())
}

// TODO: Figure out if the RAW socket implementation can be used on any other OS.
// It seems, that would be difficult on Darwin; wondering about the other BSDs though?
#[cfg(any(target_os = "linux", target_os = "android"))]
mod raw {
    use core::ops::Deref;
    use core::pin::pin;

    use std::io::{self, ErrorKind};
    use std::os::fd::{AsFd, AsRawFd};

    #[cfg(not(feature = "async-io-mini"))]
    use async_io::Async;
    #[cfg(feature = "async-io-mini")]
    use async_io_mini::Async;

    use edge_nal::{MacAddr, RawBind, RawReceive, RawSend, RawSplit, Readable};
    use embedded_io_async::ErrorType;

    use crate::sys;
    use crate::syscall_los;

    /// The RAW interface binding type for Linux.
    #[derive(Default, Copy, Clone)]
    pub struct Interface(u32);

    impl Interface {
        /// Create a new RAW interface binding for the given interface index.
        pub const fn new(interface: u32) -> Self {
            Self(interface)
        }
    }

    impl RawBind for Interface {
        type Error = io::Error;

        type Socket<'a>
            = RawSocket
        where
            Self: 'a;

        async fn bind(&self) -> Result<Self::Socket<'_>, Self::Error> {
            let socket = syscall_los!(unsafe {
                sys::socket(
                    sys::PF_PACKET,
                    sys::SOCK_DGRAM,
                    (sys::ETH_P_IP as u16).to_be() as _,
                )
            })?;

            let sockaddr = sys::sockaddr_ll {
                sll_family: sys::AF_PACKET as _,
                sll_protocol: (sys::ETH_P_IP as u16).to_be() as _,
                sll_ifindex: self.0 as _,
                sll_hatype: 0,
                sll_pkttype: 0,
                sll_halen: 0,
                sll_addr: Default::default(),
            };

            syscall_los!(unsafe {
                sys::bind(
                    socket,
                    &sockaddr as *const _ as *const _,
                    core::mem::size_of::<sys::sockaddr_ll>() as _,
                )
            })?;

            // TODO
            // syscall_los!(unsafe {
            //     sys::setsockopt(socket, sys::SOL_PACKET, sys::PACKET_AUXDATA, &1_u32 as *const _ as *const _, 4)
            // })?;

            let socket = {
                use std::os::fd::FromRawFd;

                unsafe { std::net::UdpSocket::from_raw_fd(socket) }
            };

            socket.set_broadcast(true)?;

            Ok(RawSocket(Async::new(socket)?, self.0 as _))
        }
    }

    /// The RAW socket type for Linux.
    pub struct RawSocket(Async<std::net::UdpSocket>, u32);

    impl RawSocket {
        /// Create a new RAW socket from the given async UDP socket and interface index.
        ///
        /// # Arguments
        /// - `socket`: The async UDP socket to wrap.
        /// - `interface`: The interface index.
        pub const fn new(socket: Async<std::net::UdpSocket>, interface: u32) -> Self {
            Self(socket, interface)
        }

        /// Release the underlying async UDP socket and interface index.
        pub fn release(self) -> (Async<std::net::UdpSocket>, u32) {
            (self.0, self.1)
        }
    }

    impl Deref for RawSocket {
        type Target = Async<std::net::UdpSocket>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl ErrorType for &RawSocket {
        type Error = io::Error;
    }

    impl RawReceive for &RawSocket {
        async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, MacAddr), Self::Error> {
            let fut = pin!(self.0.read_with(|io| {
                let mut storage: sys::sockaddr_storage = unsafe { core::mem::zeroed() };
                let mut addrlen = core::mem::size_of_val(&storage) as sys::socklen_t;

                let ret = syscall_los!(unsafe {
                    sys::recvfrom(
                        io.as_fd().as_raw_fd(),
                        buffer.as_mut_ptr() as *mut _,
                        buffer.len(),
                        0,
                        &mut storage as *mut _ as *mut _,
                        &mut addrlen,
                    )
                })?;

                let sockaddr = as_sockaddr_ll(&storage, addrlen as usize)?;

                let mut mac = [0; 6];
                mac.copy_from_slice(&sockaddr.sll_addr[..6]);

                Ok((ret as usize, mac))
            }));

            fut.await
        }
    }

    impl RawSend for &RawSocket {
        async fn send(&mut self, mac: MacAddr, data: &[u8]) -> Result<(), Self::Error> {
            let mut sockaddr = sys::sockaddr_ll {
                sll_family: sys::AF_PACKET as _,
                sll_protocol: (sys::ETH_P_IP as u16).to_be() as _,
                sll_ifindex: self.1 as _,
                sll_hatype: 0,
                sll_pkttype: 0,
                sll_halen: 0,
                sll_addr: Default::default(),
            };

            sockaddr.sll_halen = mac.len() as _;
            sockaddr.sll_addr[..mac.len()].copy_from_slice(&mac);

            let fut = pin!(self.0.write_with(|io| {
                let len = core::cmp::min(data.len(), u16::MAX as usize);

                let ret = syscall_los!(unsafe {
                    sys::sendto(
                        io.as_fd().as_raw_fd(),
                        data.as_ptr() as *const _,
                        len,
                        sys::MSG_NOSIGNAL,
                        &sockaddr as *const _ as *const _,
                        core::mem::size_of::<sys::sockaddr_ll>() as _,
                    )
                })?;
                Ok(ret as usize)
            }));

            let len = fut.await?;

            assert_eq!(len, data.len());

            Ok(())
        }
    }

    impl Readable for &RawSocket {
        async fn readable(&mut self) -> Result<(), Self::Error> {
            self.0.readable().await
        }
    }

    impl ErrorType for RawSocket {
        type Error = io::Error;
    }

    impl RawReceive for RawSocket {
        async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, MacAddr), Self::Error> {
            let mut rself = &*self;

            let fut = pin!(rself.receive(buffer));

            fut.await
        }
    }

    impl RawSend for RawSocket {
        async fn send(&mut self, mac: MacAddr, data: &[u8]) -> Result<(), Self::Error> {
            let mut rself = &*self;

            let fut = pin!(rself.send(mac, data));

            fut.await
        }
    }

    impl RawSplit for RawSocket {
        type Receive<'a>
            = &'a Self
        where
            Self: 'a;

        type Send<'a>
            = &'a Self
        where
            Self: 'a;

        fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
            let socket = &*self;

            (socket, socket)
        }
    }

    impl Readable for RawSocket {
        async fn readable(&mut self) -> Result<(), Self::Error> {
            self.0.readable().await
        }
    }

    fn as_sockaddr_ll(
        storage: &sys::sockaddr_storage,
        len: usize,
    ) -> io::Result<&sys::sockaddr_ll> {
        match storage.ss_family as core::ffi::c_int {
            sys::AF_PACKET => {
                assert!(len >= core::mem::size_of::<sys::sockaddr_ll>());
                Ok(unsafe { (storage as *const _ as *const sys::sockaddr_ll).as_ref() }.unwrap())
            }
            _ => Err(io::Error::new(ErrorKind::InvalidInput, "invalid argument")),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "espidf"))]
mod sys {
    pub use libc::*;

    #[macro_export]
    macro_rules! syscall {
        ($ret:expr) => {{
            let result = $ret;

            if result != 0 {
                Err(::std::io::Error::from_raw_os_error(result))
            } else {
                Ok(result)
            }
        }};
    }

    #[macro_export]
    macro_rules! syscall_los {
        ($ret:expr) => {{
            let result = $ret;

            if result == -1 {
                Err(::std::io::Error::last_os_error())
            } else {
                Ok(result)
            }
        }};
    }

    #[macro_export]
    macro_rules! syscall_los_eagain {
        ($ret:expr) => {{
            #[allow(unreachable_patterns)]
            match syscall_los!($ret) {
                Ok(_) => Ok(()),
                Err(e)
                    if matches!(
                        e.raw_os_error(),
                        Some(sys::EINPROGRESS) | Some(sys::EAGAIN) | Some(sys::EWOULDBLOCK)
                    ) =>
                {
                    Ok(())
                }
                Err(e) => Err(e),
            }?;

            Ok::<_, io::Error>(())
        }};
    }
}
