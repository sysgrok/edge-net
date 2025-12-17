use core::fmt;
use core::net::SocketAddr;
use core::pin::pin;
use core::ptr::NonNull;

use edge_nal::{Close, Readable, TcpBind, TcpConnect, TcpShutdown, TcpSplit};

use embassy_futures::join::join;

use embassy_net::tcp::{AcceptError, ConnectError, Error, TcpReader, TcpWriter};
use embassy_net::Stack;

use embedded_io_async::{ErrorKind, ErrorType, Read, Write};

use crate::sealed::SealedDynPool;
use crate::{to_emb_bind_socket, to_emb_socket, to_net_socket, DynPool, Pool};

/// A type that implements the `TcpConnect` and `TcpBind` factory traits from `edge-nal`
/// Uses the provided Embassy networking stack and TCP buffers pool to create TCP sockets.
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Copy, Clone)]
pub struct Tcp<'d> {
    /// The Embassy networking stack to use for creating TCP sockets.
    stack: Stack<'d>,
    /// The pool of TCP socket buffers to use for creating TCP sockets.
    buffers: &'d dyn DynPool<TcpSocketBuffers>,
}

impl<'d> Tcp<'d> {
    /// Create a new `Tcp` instance for the provided Embassy networking stack, using the provided TCP buffers.
    ///
    /// # Arguments
    /// - `stack`: The Embassy networking stack to use for creating TCP sockets.
    /// - `buffers`: A reference to a pool of TCP socket buffers.
    ///   NOTE: Ensure that the number of buffers in the pool is not greater than the number of sockets
    ///   supported by the provided [embassy_net::Stack], or else [smoltcp::iface::SocketSet] will panic with
    ///   `adding a socket to a full SocketSet`.
    pub fn new(stack: Stack<'d>, buffers: &'d dyn DynPool<TcpSocketBuffers>) -> Self {
        Self { stack, buffers }
    }
}

impl TcpConnect for Tcp<'_> {
    type Error = TcpError;

    type Socket<'a>
        = TcpSocket<'a>
    where
        Self: 'a;

    async fn connect(&self, remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
        let mut socket = TcpSocket::new(self.stack, self.buffers)?;

        socket
            .socket
            .connect(to_emb_socket(remote).ok_or(TcpError::UnsupportedProto)?)
            .await?;

        Ok(socket)
    }
}

impl TcpBind for Tcp<'_> {
    type Error = TcpError;

    type Accept<'a>
        = TcpAccept<'a>
    where
        Self: 'a;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Accept<'_>, Self::Error> {
        Ok(TcpAccept {
            stack: *self,
            local,
        })
    }
}

/// A type that represents an acceptor for incoming TCP client connections.
/// Implements the `TcpAccept` factory trait from `edge-nal`
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Copy, Clone)]
pub struct TcpAccept<'d> {
    stack: Tcp<'d>,
    local: SocketAddr,
}

impl edge_nal::TcpAccept for TcpAccept<'_> {
    type Error = TcpError;

    type Socket<'a>
        = TcpSocket<'a>
    where
        Self: 'a;

    async fn accept(&self) -> Result<(SocketAddr, Self::Socket<'_>), Self::Error> {
        let mut socket = TcpSocket::new(self.stack.stack, self.stack.buffers)?;

        socket
            .socket
            .accept(to_emb_bind_socket(self.local).ok_or(TcpError::UnsupportedProto)?)
            .await?;

        let local_endpoint = unwrap!(socket.socket.local_endpoint());

        Ok((to_net_socket(local_endpoint), socket))
    }
}

/// A type that represents a TCP socket
/// Implements the `Read` and `Write` traits from `embedded-io-async`, as well as the `TcpSplit` factory trait from `edge-nal`
pub struct TcpSocket<'d> {
    /// The underlying Embassy TCP socket.
    socket: embassy_net::tcp::TcpSocket<'d>,
    /// The pool of TCP socket buffers used by this socket.
    stack_buffers: &'d dyn DynPool<TcpSocketBuffers>,
    /// The token used to identify the socket buffers in the pool.
    buffer_token: NonNull<u8>,
}

impl<'d> TcpSocket<'d> {
    fn new(
        stack: Stack<'d>,
        stack_buffers: &'d dyn DynPool<TcpSocketBuffers>,
    ) -> Result<Self, TcpError> {
        let mut socket_buffers = stack_buffers.alloc().ok_or(TcpError::NoBuffers)?;

        Ok(Self {
            socket: embassy_net::tcp::TcpSocket::new(
                stack,
                unsafe {
                    core::slice::from_raw_parts_mut(
                        socket_buffers.rx_buf.as_mut(),
                        socket_buffers.rx_buf_len,
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

    async fn close(&mut self, what: Close) -> Result<(), TcpError> {
        async fn discard_all_data(rx: &mut TcpReader<'_>) -> Result<(), TcpError> {
            let mut buf = [0; 32];

            while rx.read(&mut buf).await? > 0 {}

            Ok(())
        }

        if matches!(what, Close::Both | Close::Write) {
            self.socket.close();
        }

        let (mut rx, mut tx) = self.socket.split();

        match what {
            Close::Read => discard_all_data(&mut rx).await?,
            Close::Write => tx.flush().await?,
            Close::Both => {
                let mut flush = pin!(tx.flush());
                let mut read = pin!(discard_all_data(&mut rx));

                match join(&mut flush, &mut read).await {
                    (Err(e), _) => Err(e)?,
                    (_, Err(e)) => Err(e)?,
                    _ => (),
                }
            }
        }

        Ok(())
    }

    async fn abort(&mut self) -> Result<(), TcpError> {
        self.socket.abort();
        self.socket.flush().await?;

        Ok(())
    }
}

impl Drop for TcpSocket<'_> {
    fn drop(&mut self) {
        self.socket.close();
        unsafe {
            self.stack_buffers.free(self.buffer_token);
        }
    }
}

impl ErrorType for TcpSocket<'_> {
    type Error = TcpError;
}

impl Read for TcpSocket<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.socket.read(buf).await?)
    }
}

impl Write for TcpSocket<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(self.socket.write(buf).await?)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.socket.flush().await?;

        Ok(())
    }
}

impl Readable for TcpSocket<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.wait_read_ready().await;
        Ok(())
    }
}

impl TcpShutdown for TcpSocket<'_> {
    async fn close(&mut self, what: Close) -> Result<(), Self::Error> {
        TcpSocket::close(self, what).await
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        TcpSocket::abort(self).await
    }
}

/// A type that represents the read half of a split TCP socket.
/// Implements the `Read` trait from `embedded-io-async`.
pub struct TcpSocketRead<'a>(TcpReader<'a>);

impl ErrorType for TcpSocketRead<'_> {
    type Error = TcpError;
}

impl Read for TcpSocketRead<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await.map_err(TcpError::from)
    }
}

impl Readable for TcpSocketRead<'_> {
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.0.wait_read_ready().await;
        Ok(())
    }
}

/// A type that represents the write half of a split TCP socket.
/// Implements the `Write` trait from `embedded-io-async`.
pub struct TcpSocketWrite<'a>(TcpWriter<'a>);

impl ErrorType for TcpSocketWrite<'_> {
    type Error = TcpError;
}

impl Write for TcpSocketWrite<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await.map_err(TcpError::from)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(TcpError::from)
    }
}

impl TcpSplit for TcpSocket<'_> {
    type Read<'a>
        = TcpSocketRead<'a>
    where
        Self: 'a;

    type Write<'a>
        = TcpSocketWrite<'a>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
        let (read, write) = self.socket.split();

        (TcpSocketRead(read), TcpSocketWrite(write))
    }
}

/// A shared error type that is used by the TCP factory traits implementation as well as the TCP socket.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TcpError {
    /// A general TCP error.
    General(Error),
    /// An error that occurred while connecting.
    Connect(ConnectError),
    /// An error that occurred while accepting a connection.
    Accept(AcceptError),
    /// No TCP socket buffers available.
    NoBuffers,
    /// The provided socket address uses an unsupported protocol.
    UnsupportedProto,
}

impl From<Error> for TcpError {
    fn from(e: Error) -> Self {
        TcpError::General(e)
    }
}

impl From<ConnectError> for TcpError {
    fn from(e: ConnectError) -> Self {
        TcpError::Connect(e)
    }
}

impl From<AcceptError> for TcpError {
    fn from(e: AcceptError) -> Self {
        TcpError::Accept(e)
    }
}

impl fmt::Display for TcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::General(e) => write!(f, "General: {:?}", e),
            Self::Connect(e) => write!(f, "Connect: {:?}", e),
            Self::Accept(e) => write!(f, "Accept: {:?}", e),
            Self::NoBuffers => write!(f, "No buffers"),
            Self::UnsupportedProto => write!(f, "Unsupported protocol"),
        }
    }
}

impl core::error::Error for TcpError {}

impl embedded_io_async::Error for TcpError {
    fn kind(&self) -> ErrorKind {
        match self {
            TcpError::General(_) => ErrorKind::Other,
            TcpError::Connect(_) => ErrorKind::Other,
            TcpError::Accept(_) => ErrorKind::Other,
            TcpError::NoBuffers => ErrorKind::OutOfMemory,
            TcpError::UnsupportedProto => ErrorKind::InvalidInput,
        }
    }
}

/// A type that holds the TCP socket buffers.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TcpSocketBuffers {
    /// A token used to identify the buffer in the pool.
    token: NonNull<u8>,
    /// The buffer for receiving data.
    rx_buf: NonNull<u8>,
    /// The buffer for transmitting data.
    tx_buf: NonNull<u8>,
    /// The length of the receive buffer.
    rx_buf_len: usize,
    /// The length of the transmit buffer.
    tx_buf_len: usize,
}

/// A type alias for a pool of TCP socket buffers.
pub type TcpBuffers<const N: usize, const TX_SZ: usize = 1024, const RX_SZ: usize = 1024> =
    Pool<([u8; TX_SZ], [u8; RX_SZ]), N>;

impl<const N: usize, const TX_SZ: usize, const RX_SZ: usize> SealedDynPool<TcpSocketBuffers>
    for TcpBuffers<N, TX_SZ, RX_SZ>
{
    fn alloc(&self) -> Option<TcpSocketBuffers> {
        let mut socket_buffers = Pool::alloc(self)?;

        let rx_buf = unsafe { &mut socket_buffers.as_mut().1 };
        let tx_buf = unsafe { &mut socket_buffers.as_mut().0 };

        Some(TcpSocketBuffers {
            token: socket_buffers.cast::<u8>(),
            rx_buf: unwrap!(NonNull::new(rx_buf.as_mut_ptr())),
            tx_buf: unwrap!(NonNull::new(tx_buf.as_mut_ptr())),
            rx_buf_len: rx_buf.len(),
            tx_buf_len: tx_buf.len(),
        })
    }

    unsafe fn free(&self, buffer_token: NonNull<u8>) {
        unsafe {
            Pool::free(self, buffer_token.cast::<([u8; TX_SZ], [u8; RX_SZ])>());
        }
    }
}

impl<const N: usize, const TX_SZ: usize, const RX_SZ: usize> DynPool<TcpSocketBuffers>
    for TcpBuffers<N, TX_SZ, RX_SZ>
{
}
