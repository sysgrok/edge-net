use core::future::poll_fn;
use core::pin::pin;
use core::ptr::NonNull;

use edge_nal::{MacAddr, RawBind, RawReceive, RawSend, RawSplit, Readable};
use embassy_net::raw::{PacketMetadata, RecvError};
use embassy_net::Stack;
use embedded_io_async::ErrorType;

use crate::sealed::SealedDynPool;
use crate::{to_emb_bind_socket, to_emb_socket, to_net_socket, DynPool, Pool};

/// A type that implements the `RawBind` factory trait from `edge-nal`.
/// Uses the provided Embassy networking stack and raw buffers pool to create raw sockets.
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Copy, Clone)]
pub struct Raw<'d> {
    /// The Embassy networking stack to use for creating raw sockets.
    stack: Stack<'d>,
    /// The pool of raw socket buffers to use for creating raw sockets.
    buffers: &'d dyn DynPool<RawSocketBuffers>,
}

impl<'d> Raw<'d> {
    /// Create a new `Raw` instance for the provided Embassy networking stack using the provided raw buffers.
    ///
    /// # Arguments
    /// - `stack`: The Embassy networking stack to use for creating UDP sockets.
    /// - `buffers`: A pool of UDP socket buffers to use for creating UDP sockets.
    ///   NOTE: Ensure that the number of buffers in the pool is not greater than the number of sockets
    ///   supported by the provided [embassy_net::Stack], or else [smoltcp::iface::SocketSet] will panic with
    ///   `adding a socket to a full SocketSet`.
    pub fn new(stack: Stack<'d>, buffers: &'d dyn DynPool<RawSocketBuffers>) -> Self {
        Self { stack, buffers }
    }
}

impl RawBind for Raw<'_> {
    type Error = RawError;

    type Socket<'a>
        = RawSocket<'a>
    where
        Self: 'a;

    async fn bind(&self) -> Result<Self::Socket<'_>, Self::Error> {
        let socket = RawSocket::new(self.stack, self.buffers, None, None)?;
        Ok(socket)
    }
}

/// A raw socket.
/// Implements the `RawReceive` `RawSend` and `RawSplit` traits from `edge-nal`.
pub struct RawSocket<'d> {
    /// The Embassy networking stack.
    #[allow(unused)]
    stack: embassy_net::Stack<'d>,
    /// The underlying Embassy raw socket.
    socket: embassy_net::raw::RawSocket<'d>,
    /// The pool of raw socket buffers.
    stack_buffers: &'d dyn DynPool<RawSocketBuffers>,
    /// The token used to identify the socket buffers in the pool.
    buffer_token: NonNull<u8>,
}

impl<'d> RawSocket<'d> {
    fn new(
        stack: Stack<'d>,
        stack_buffers: &'d dyn DynPool<RawSocketBuffers>,
        version: Option<embassy_net::raw::IpVersion>,
        protocol: Option<embassy_net::raw::IpProtocol>,
    ) -> Result<Self, RawError> {
        let mut socket_buffers = stack_buffers.alloc().ok_or(RawError::NoBuffers)?;

        Ok(Self {
            stack,
            socket: embassy_net::raw::RawSocket::new(
                stack,
                version,
                protocol,
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

impl ErrorType for &RawSocket<'_> {
    type Error = RawError;
}

impl RawReceive for &RawSocket<'_> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, MacAddr), Self::Error> {
        let result = poll_fn(|cx| self.socket.poll_recv(buffer, cx)).await?;

        Ok((result, [0; 6]))
    }
}

impl RawSend for &RawSocket<'_> {
    async fn send(&mut self, _mac: MacAddr, data: &[u8]) -> Result<(), Self::Error> {
        poll_fn(|cx| self.socket.poll_send(data, cx)).await;
        Ok(())
    }
}

impl Readable for &RawSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.wait_recv_ready().await;
        Ok(())
    }
}

impl Drop for RawSocket<'_> {
    fn drop(&mut self) {
        unsafe {
            self.stack_buffers.free(self.buffer_token);
        }
    }
}

impl ErrorType for RawSocket<'_> {
    type Error = RawError;
}

impl RawReceive for RawSocket<'_> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, MacAddr), Self::Error> {
        let mut rself = &*self;

        let fut = pin!(rself.receive(buffer));

        fut.await
    }
}

impl RawSend for RawSocket<'_> {
    async fn send(&mut self, mac: MacAddr, data: &[u8]) -> Result<(), Self::Error> {
        let mut rself = &*self;

        let fut = pin!(rself.send(mac, data));

        fut.await
    }
}

impl Readable for RawSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.wait_recv_ready().await;
        Ok(())
    }
}

impl RawSplit for RawSocket<'_> {
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

/// A shared error type that is used by the UDP factory trait implementation as well as the UDP socket
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RawError {
    /// An error occurred while receiving data.
    Recv(RecvError),
    /// No more raw socket buffers are available.
    NoBuffers,
}

impl From<RecvError> for RawError {
    fn from(e: RecvError) -> Self {
        RawError::Recv(e)
    }
}

impl core::fmt::Display for RawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RawError::Recv(e) => write!(f, "Raw receive error: {:?}", e),
            RawError::NoBuffers => write!(f, "No raw socket buffers available"),
        }
    }
}

impl core::error::Error for RawError {}
impl embedded_io_async::Error for RawError {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            RawError::NoBuffers => embedded_io_async::ErrorKind::OutOfMemory,
            _ => embedded_io_async::ErrorKind::Other,
        }
    }
}

/// A type that holds the UDP socket buffers.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RawSocketBuffers {
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
pub type RawBuffers<
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
    SealedDynPool<RawSocketBuffers> for RawBuffers<N, TX_SZ, RX_SZ, M>
{
    fn alloc(&self) -> Option<RawSocketBuffers> {
        let mut socket_buffers = Pool::alloc(self)?;

        let rx_buf = unsafe { &mut socket_buffers.as_mut().1 };
        let tx_buf = unsafe { &mut socket_buffers.as_mut().0 };
        let md_rx_buf = unsafe { &mut socket_buffers.as_mut().3 };
        let md_tx_buf = unsafe { &mut socket_buffers.as_mut().2 };

        Some(RawSocketBuffers {
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
    DynPool<RawSocketBuffers> for RawBuffers<N, TX_SZ, RX_SZ, M>
{
}
