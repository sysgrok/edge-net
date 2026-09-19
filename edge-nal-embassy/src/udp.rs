use core::fmt::Display;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use core::ptr::NonNull;

use edge_nal::{
    MulticastV4, MulticastV6, Readable, UdpBind, UdpReceive, UdpSend, UdpSplit, UdpSplitMulticast,
};

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
        let socket_buffers = stack_buffers.alloc().ok_or(UdpError::NoBuffers)?;

        // SAFETY: The buffers are exclusively owned by this socket until `free` is called from
        // `Drop`, and the slices are built from the raw pointers (rather than from `&mut`
        // references to their first element) so that they cover the whole buffers.
        Ok(Self {
            stack,
            socket: embassy_net::udp::UdpSocket::new(
                stack,
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.md_rx_buf.as_ptr(),
                        socket_buffers.md_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.rx_buf.as_ptr(),
                        socket_buffers.rx_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.md_tx_buf.as_ptr(),
                        socket_buffers.md_buf_len,
                    )
                },
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.tx_buf.as_ptr(),
                        socket_buffers.tx_buf_len,
                    )
                },
            ),
            stack_buffers,
            buffer_token: socket_buffers.token,
        })
    }

    async fn join_v4(
        &self,
        #[allow(unused)] multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), UdpError> {
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
        &self,
        #[allow(unused)] multicast_addr: Ipv4Addr,
        _interface: Ipv4Addr,
    ) -> Result<(), UdpError> {
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

    async fn join_v6(
        &self,
        #[allow(unused)] multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), UdpError> {
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
        &self,
        #[allow(unused)] multicast_addr: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), UdpError> {
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

impl<'d> UdpSplitMulticast for UdpSocket<'d> {
    type MulticastV4<'a>
        = &'a Self
    where
        Self: 'a;

    type MulticastV6<'a>
        = &'a Self
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
        (&*self, &*self, &*self, &*self)
    }
}

impl MulticastV4 for UdpSocket<'_> {
    async fn join_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        Self::join_v4(self, multicast_addr, interface).await
    }

    async fn leave_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        Self::leave_v4(self, multicast_addr, interface).await
    }
}

impl MulticastV4 for &UdpSocket<'_> {
    async fn join_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        UdpSocket::join_v4(self, multicast_addr, interface).await
    }

    async fn leave_v4(
        &mut self,
        multicast_addr: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Result<(), Self::Error> {
        UdpSocket::leave_v4(self, multicast_addr, interface).await
    }
}

impl MulticastV6 for UdpSocket<'_> {
    async fn join_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        Self::join_v6(self, multicast_addr, interface).await
    }

    async fn leave_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        Self::leave_v6(self, multicast_addr, interface).await
    }
}

impl MulticastV6 for &UdpSocket<'_> {
    async fn join_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        UdpSocket::join_v6(self, multicast_addr, interface).await
    }

    async fn leave_v6(
        &mut self,
        multicast_addr: Ipv6Addr,
        interface: u32,
    ) -> Result<(), Self::Error> {
        UdpSocket::leave_v6(self, multicast_addr, interface).await
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

impl Display for UdpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UdpError::Recv(e) => write!(f, "UDP receive error: {:?}", e),
            UdpError::Send(e) => write!(f, "UDP send error: {:?}", e),
            UdpError::Bind(e) => write!(f, "UDP bind error: {:?}", e),
            UdpError::MulticastGroupTableFull => {
                write!(f, "UDP multicast group table is full")
            }
            UdpError::MulticastUnaddressable => {
                write!(f, "UDP multicast address is unaddressable")
            }
            UdpError::NoBuffers => write!(f, "No UDP socket buffers available"),
            UdpError::UnsupportedProto => write!(f, "Unsupported protocol"),
        }
    }
}

impl core::error::Error for UdpError {}

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
        let socket_buffers = Pool::alloc(self)?;

        // The buffers are addressed via raw pointers projected from the pool slot rather than via
        // references to the slot as a whole: a reference to the whole slot would alias - and under
        // the aliasing rules invalidate - the other buffers handed out from the same slot.
        //
        // SAFETY: `socket_buffers` points to a live slot of the pool.
        let (tx_buf, rx_buf, md_tx_buf, md_rx_buf) = unsafe {
            let slot = socket_buffers.as_ptr();

            (
                core::ptr::addr_of_mut!((*slot).0) as *mut u8,
                core::ptr::addr_of_mut!((*slot).1) as *mut u8,
                core::ptr::addr_of_mut!((*slot).2) as *mut PacketMetadata,
                core::ptr::addr_of_mut!((*slot).3) as *mut PacketMetadata,
            )
        };

        Some(UdpSocketBuffers {
            token: socket_buffers.cast::<u8>(),
            md_rx_buf: unwrap!(NonNull::new(md_rx_buf)),
            rx_buf: unwrap!(NonNull::new(rx_buf)),
            md_tx_buf: unwrap!(NonNull::new(md_tx_buf)),
            tx_buf: unwrap!(NonNull::new(tx_buf)),
            md_buf_len: M,
            rx_buf_len: RX_SZ,
            tx_buf_len: TX_SZ,
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

#[cfg(test)]
mod tests {
    use embassy_net::udp::PacketMetadata;
    use embassy_net::StackResources;

    use crate::sealed::SealedDynPool;

    use super::{UdpBuffers, UdpSocket};

    #[test]
    fn buffers_alloc_free() {
        let pool = UdpBuffers::<2, 8, 4, 2>::new();

        let a = SealedDynPool::alloc(&pool).unwrap();
        let b = SealedDynPool::alloc(&pool).unwrap();
        assert!(SealedDynPool::alloc(&pool).is_none());

        assert_eq!(a.tx_buf_len, 8);
        assert_eq!(a.rx_buf_len, 4);
        assert_eq!(a.md_buf_len, 2);

        // SAFETY: the pointers come from live allocations of the pool, with the given lengths
        unsafe {
            let a_tx = core::slice::from_raw_parts_mut(a.tx_buf.as_ptr(), a.tx_buf_len);
            let a_rx = core::slice::from_raw_parts_mut(a.rx_buf.as_ptr(), a.rx_buf_len);
            let a_md_tx = core::slice::from_raw_parts_mut(a.md_tx_buf.as_ptr(), a.md_buf_len);
            let a_md_rx = core::slice::from_raw_parts_mut(a.md_rx_buf.as_ptr(), a.md_buf_len);
            let b_tx = core::slice::from_raw_parts_mut(b.tx_buf.as_ptr(), b.tx_buf_len);
            let b_rx = core::slice::from_raw_parts_mut(b.rx_buf.as_ptr(), b.rx_buf_len);
            let b_md_tx = core::slice::from_raw_parts_mut(b.md_tx_buf.as_ptr(), b.md_buf_len);
            let b_md_rx = core::slice::from_raw_parts_mut(b.md_rx_buf.as_ptr(), b.md_buf_len);

            // All eight buffers are live at the same time and must not overlap
            a_tx.fill(1);
            a_rx.fill(2);
            a_md_tx.fill(PacketMetadata::EMPTY);
            a_md_rx.fill(PacketMetadata::EMPTY);
            b_tx.fill(3);
            b_rx.fill(4);
            b_md_tx.fill(PacketMetadata::EMPTY);
            b_md_rx.fill(PacketMetadata::EMPTY);
            assert!(a_tx.iter().all(|v| *v == 1));
            assert!(a_rx.iter().all(|v| *v == 2));
            assert!(b_tx.iter().all(|v| *v == 3));
            assert!(b_rx.iter().all(|v| *v == 4));
            assert_eq!(
                a_md_tx.len() + a_md_rx.len() + b_md_tx.len() + b_md_rx.len(),
                8
            );

            SealedDynPool::free(&pool, a.token);
        }

        // A freed slot is handed out again
        let c = SealedDynPool::alloc(&pool).unwrap();
        assert_eq!(c.token, a.token);
        assert!(SealedDynPool::alloc(&pool).is_none());

        // SAFETY: `b` and `c` are live allocations of the pool
        unsafe {
            SealedDynPool::free(&pool, b.token);
            SealedDynPool::free(&pool, c.token);
        }

        assert!(SealedDynPool::alloc(&pool).is_some());
    }

    #[test]
    fn socket_new_and_drop() {
        let mut resources = StackResources::<2>::new();
        let (stack, _runner) = crate::tests::stack(&mut resources);

        let pool = UdpBuffers::<1, 16, 16, 2>::new();

        let Ok(socket) = UdpSocket::new(stack, &pool) else {
            panic!("socket creation failed");
        };

        // The only slot of the pool is taken by the socket
        assert!(UdpSocket::new(stack, &pool).is_err());

        // Dropping the socket returns its buffers to the pool
        drop(socket);
        assert!(UdpSocket::new(stack, &pool).is_ok());
    }
}
