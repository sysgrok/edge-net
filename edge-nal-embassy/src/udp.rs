use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use core::ptr::NonNull;

use edge_nal::{MulticastV4, MulticastV6, Readable, UdpBind, UdpReceive, UdpSend, UdpSplit};

use embassy_net::udp::{BindError, PacketMetadata, RecvError, SendError};
use embassy_net::Stack;

use embedded_io_async::{ErrorKind, ErrorType};

use crate::sealed::SealedDynPool;
use crate::{to_emb_bind_socket, to_emb_socket, to_net_socket, DynPool, Pool};

/// A type that implements the `UdpBind` factory trait from `edge-nal`.
/// Uses the provided Embassy networking stack and UDP buffers pool to create UDP sockets.
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Copy, Clone)]
pub struct Udp<'d> {
    /// The Embassy networking stack to use for creating UDP sockets.
    stack: Stack<'d>,
    /// The pool of UDP socket buffers to use for creating UDP sockets.
    buffers: &'d dyn DynPool<UdpSocketBuffers>,
}

impl<'d> Udp<'d> {
    /// Create a new `Udp` instance for the provided Embassy networking stack using the provided UDP buffers.
    ///
    /// # Arguments
    /// - `stack`: The Embassy networking stack to use for creating UDP sockets.
    /// - `buffers`: A pool of UDP socket buffers to use for creating UDP sockets.
    ///   NOTE: Ensure that the number of buffers in the pool is not greater than the number of sockets
    ///   supported by the provided [embassy_net::Stack], or else [smoltcp::iface::SocketSet] will panic with
    ///   `adding a socket to a full SocketSet`.
    pub fn new(stack: Stack<'d>, buffers: &'d dyn DynPool<UdpSocketBuffers>) -> Self {
        Self { stack, buffers }
    }
}

impl UdpBind for Udp<'_> {
    type Error = UdpError;

    type Socket<'a>
        = UdpSocket<'a>
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        let mut socket = UdpSocket::new(self.stack, self.buffers)?;

        socket
            .socket
            .bind(to_emb_bind_socket(local).ok_or(UdpError::UnsupportedProto)?)?;

        Ok(socket)
    }
}

/// A UDP socket.
/// Implements the `UdpReceive` `UdpSend` and `UdpSplit` traits from `edge-nal`.
pub struct UdpSocket<'d> {
    /// The Embassy networking stack.
    #[allow(unused)]
    stack: embassy_net::Stack<'d>,
    /// The underlying Embassy UDP socket.
    socket: embassy_net::udp::UdpSocket<'d>,
    /// The pool of UDP socket buffers.
    stack_buffers: &'d dyn DynPool<UdpSocketBuffers>,
    /// The token used to identify the socket buffers in the pool.
    buffer_token: NonNull<u8>,
}

impl<'d> UdpSocket<'d> {
    fn new(
        stack: Stack<'d>,
        stack_buffers: &'d dyn DynPool<UdpSocketBuffers>,
    ) -> Result<Self, UdpError> {
        let mut socket_buffers = stack_buffers.alloc().ok_or(UdpError::NoBuffers)?;

        Ok(Self {
            stack,
            socket: embassy_net::udp::UdpSocket::new(
                stack,
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.md_rx_buf.as_mut(),
                        socket_buffers.md_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.rx_buf.as_mut(),
                        socket_buffers.rx_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.md_tx_buf.as_mut(),
                        socket_buffers.md_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.tx_buf.as_mut(),
                        socket_buffers.tx_buf_len,
                    )
                },
            ),
            stack_buffers,
            buffer_token: socket_buffers.token,
        })
    }
}

impl Drop for UdpSocket<'_> {
    fn drop(&mut self) {
        self.socket.close();
        unsafe {
            self.stack_buffers.free(self.buffer_token);
        }
    }
}

impl ErrorType for UdpSocket<'_> {
    type Error = UdpError;
}

impl UdpReceive for UdpSocket<'_> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let (len, remote_endpoint) = self.socket.recv_from(buffer).await?;

        Ok((len, to_net_socket(remote_endpoint.endpoint)))
    }
}

impl UdpSend for UdpSocket<'_> {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        self.socket
            .send_to(
                data,
                to_emb_socket(remote).ok_or(UdpError::UnsupportedProto)?,
            )
            .await?;

        Ok(())
    }
}

impl ErrorType for &UdpSocket<'_> {
    type Error = UdpError;
}

impl UdpReceive for &UdpSocket<'_> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let (len, remote_endpoint) = self.socket.recv_from(buffer).await?;

        Ok((len, to_net_socket(remote_endpoint.endpoint)))
    }
}

impl UdpSend for &UdpSocket<'_> {
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        self.socket
            .send_to(
                data,
                to_emb_socket(remote).ok_or(UdpError::UnsupportedProto)?,
            )
            .await?;

        Ok(())
    }
}

impl Readable for &UdpSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.wait_recv_ready().await;
        Ok(())
    }
}

impl UdpSplit for UdpSocket<'_> {
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

impl MulticastV4 for UdpSocket<'_> {
    async fn join_v4(
        &mut self,
        #[allow(unused)] multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        #[cfg(feature = "multicast")]
        {
            self.stack.join_multicast_group(
                crate::to_emb_addr(core::net::IpAddr::V4(multicast_addr))
                    .ok_or(UdpError::UnsupportedProto)?,
            )?;
        }

        #[cfg(not(feature = "multicast"))]
        {
            Err(UdpError::UnsupportedProto)?;
        }

        Ok(())
    }

    async fn leave_v4(
        &mut self,
        #[allow(unused)] multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        #[cfg(feature = "multicast")]
        {
            self.stack.leave_multicast_group(
                crate::to_emb_addr(core::net::IpAddr::V4(multicast_addr))
                    .ok_or(UdpError::UnsupportedProto)?,
            )?;
        }

        #[cfg(not(feature = "multicast"))]
        {
            Err(UdpError::UnsupportedProto)?;
        }

        Ok(())
    }
}

impl MulticastV6 for UdpSocket<'_> {
    async fn join_v6(
        &mut self,
        #[allow(unused)] multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        #[cfg(feature = "multicast")]
        {
            self.stack.join_multicast_group(
                crate::to_emb_addr(core::net::IpAddr::V6(multicast_addr))
                    .ok_or(UdpError::UnsupportedProto)?,
            )?;
        }

        #[cfg(not(feature = "multicast"))]
        {
            Err(UdpError::UnsupportedProto)?;
        }

        Ok(())
    }

    async fn leave_v6(
        &mut self,
        #[allow(unused)] multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Self::Error> {
        #[cfg(feature = "multicast")]
        {
            self.stack.leave_multicast_group(
                crate::to_emb_addr(core::net::IpAddr::V6(multicast_addr))
                    .ok_or(UdpError::UnsupportedProto)?,
            )?;
        }

        #[cfg(not(feature = "multicast"))]
        {
            Err(UdpError::UnsupportedProto)?;
        }

        Ok(())
    }
}

impl Readable for UdpSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.wait_recv_ready().await;
        Ok(())
    }
}

/// A shared error type that is used by the UDP factory trait implementation as well as the UDP socket
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum UdpError {
    /// An error occurred while receiving data.
    Recv(RecvError),
    /// An error occurred while sending data.
    Send(SendError),
    /// An error occurred while binding the socket.
    Bind(BindError),
    /// The table of joined multicast groups is already full.
    MulticastGroupTableFull,
    /// Cannot join/leave the given multicast group.
    MulticastUnaddressable,
    /// No more UDP socket buffers are available.
    NoBuffers,
    /// The provided protocol is not supported.
    UnsupportedProto,
}

impl From<RecvError> for UdpError {
    fn from(e: RecvError) -> Self {
        UdpError::Recv(e)
    }
}

impl From<SendError> for UdpError {
    fn from(e: SendError) -> Self {
        UdpError::Send(e)
    }
}

impl From<BindError> for UdpError {
    fn from(e: BindError) -> Self {
        UdpError::Bind(e)
    }
}

#[cfg(all(
    feature = "multicast",
    any(feature = "proto-ipv4", feature = "proto-ipv6")
))]
impl From<embassy_net::MulticastError> for UdpError {
    fn from(e: embassy_net::MulticastError) -> Self {
        match e {
            embassy_net::MulticastError::GroupTableFull => UdpError::MulticastGroupTableFull,
            embassy_net::MulticastError::Unaddressable => UdpError::MulticastUnaddressable,
        }
    }
}

impl embedded_io_async::Error for UdpError {
    fn kind(&self) -> ErrorKind {
        match self {
            UdpError::Recv(_) => ErrorKind::Other,
            UdpError::Send(_) => ErrorKind::Other,
            UdpError::Bind(_) => ErrorKind::Other,
            UdpError::MulticastGroupTableFull => ErrorKind::Other,
            UdpError::MulticastUnaddressable => ErrorKind::Other,
            UdpError::NoBuffers => ErrorKind::OutOfMemory,
            UdpError::UnsupportedProto => ErrorKind::InvalidInput,
        }
    }
}

/// A type that holds the UDP socket buffers.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct UdpSocketBuffers {
    /// A token used to identify the buffer in the pool.
    token: NonNull<u8>,
    /// The metadata buffer for receiving packets.
    md_rx_buf: NonNull<PacketMetadata>,
    /// The buffer for receiving packets.
    rx_buf: NonNull<u8>,
    /// The metadata buffer for transmitting packets.
    md_tx_buf: NonNull<PacketMetadata>,
    /// The buffer for transmitting packets.
    tx_buf: NonNull<u8>,
    /// The length of a metadata buffer.
    md_buf_len: usize,
    /// The length of the receive buffer.
    rx_buf_len: usize,
    /// The length of the transmit buffer.
    tx_buf_len: usize,
}

/// A type alias for a pool of UDP socket buffers.
pub type UdpBuffers<
    const N: usize,
    const TX_SZ: usize = 1472,
    const RX_SZ: usize = 1472,
    const M: usize = 2,
> = Pool<
    (
        [u8; TX_SZ],
        [u8; RX_SZ],
        [PacketMetadata; M],
        [PacketMetadata; M],
    ),
    N,
>;

impl<const N: usize, const TX_SZ: usize, const RX_SZ: usize, const M: usize>
    SealedDynPool<UdpSocketBuffers> for UdpBuffers<N, TX_SZ, RX_SZ, M>
{
    fn alloc(&self) -> Option<UdpSocketBuffers> {
        let mut socket_buffers = Pool::alloc(self)?;

        let rx_buf = unsafe { &mut socket_buffers.as_mut().1 };
        let tx_buf = unsafe { &mut socket_buffers.as_mut().0 };
        let md_rx_buf = unsafe { &mut socket_buffers.as_mut().3 };
        let md_tx_buf = unsafe { &mut socket_buffers.as_mut().2 };

        Some(UdpSocketBuffers {
            token: socket_buffers.cast::<u8>(),
            md_rx_buf: unwrap!(NonNull::new(md_rx_buf.as_mut_ptr())),
            rx_buf: unwrap!(NonNull::new(rx_buf.as_mut_ptr())),
            md_tx_buf: unwrap!(NonNull::new(md_tx_buf.as_mut_ptr())),
            tx_buf: unwrap!(NonNull::new(tx_buf.as_mut_ptr())),
            md_buf_len: md_rx_buf.len(),
            rx_buf_len: rx_buf.len(),
            tx_buf_len: tx_buf.len(),
        })
    }

    unsafe fn free(&self, buffer_token: NonNull<u8>) {
        unsafe {
            Pool::free(
                self,
                buffer_token.cast::<(
                    [u8; TX_SZ],
                    [u8; RX_SZ],
                    [PacketMetadata; M],
                    [PacketMetadata; M],
                )>(),
            );
        }
    }
}

impl<const N: usize, const TX_SZ: usize, const RX_SZ: usize, const M: usize>
    DynPool<UdpSocketBuffers> for UdpBuffers<N, TX_SZ, RX_SZ, M>
{
}
