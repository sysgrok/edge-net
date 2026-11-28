//! DTLS over `edge-nal` UDP sockets.

use core::future::Future;
use core::net::SocketAddr;

use embedded_io_async::ErrorType;

use edge_nal::{Readable, UdpReceive, UdpSend};

use mbedtls_rs::dtls::{Clock, Datagram, Session};
use mbedtls_rs::SessionError;

/// A DTLS socket, layered on top of an `edge-nal` UDP socket `S`.
///
/// It implements `edge-nal`'s `UdpReceive`, `UdpSend`, `Readable` and `UdpSocket` traits, as a UDP
/// socket connected to the DTLS peer, and so can be used where such a socket is expected.
///
/// It is a thin newtype wrapper around `mbedtls-rs`'s `dtls::Session`, for the same reason as
/// [`TlsSocket`](crate::TlsSocket).
pub struct DtlsSocket<'d, S, C>(Session<'d, UdpDatagram<S>, C>)
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock;

impl<'d, S, C> DtlsSocket<'d, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
    /// Create a new instance of the `DtlsSocket` type.
    ///
    /// # Arguments:
    /// - `session` - The DTLS session, over a [`UdpDatagram`]
    pub const fn new(session: Session<'d, UdpDatagram<S>, C>) -> Self {
        Self(session)
    }

    /// Get a shared reference to the underlying `mbedtls-rs` `dtls::Session`.
    pub fn session(&self) -> &Session<'d, UdpDatagram<S>, C> {
        &self.0
    }

    /// Get a mutable reference to the underlying `mbedtls-rs` `dtls::Session`.
    pub fn session_mut(&mut self) -> &mut Session<'d, UdpDatagram<S>, C> {
        &mut self.0
    }
}

impl<S, C> ErrorType for DtlsSocket<'_, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
    type Error = SessionError;
}

impl<S, C> UdpReceive for DtlsSocket<'_, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
    /// Receive the data of one record.
    ///
    /// Unlike with a plain UDP socket, the data that does not fit in `buffer` is not dropped: the
    /// next `receive` returns it. The length is 0 also when the peer has closed the session.
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let len = self.0.read(buffer).await?;

        Ok((len, self.0.io().peer))
    }
}

impl<S, C> UdpSend for DtlsSocket<'_, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
    /// Send `data` as one record. `remote` is ignored, as the socket is connected to the peer.
    async fn send(&mut self, _remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        self.0.write(data).await?;

        Ok(())
    }
}

impl<S, C> Readable for DtlsSocket<'_, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
    async fn readable(&mut self) -> Result<(), Self::Error> {
        // With an empty buffer, MbedTLS decrypts the next record and keeps its data for the next read
        self.0.read(&mut []).await?;

        Ok(())
    }
}

impl<S, C> edge_nal::UdpSocket for DtlsSocket<'_, S, C>
where
    S: UdpReceive + UdpSend + Readable,
    C: Clock,
{
}

/// An adaptor from `edge-nal`'s UDP socket traits to `mbedtls-rs`'s `dtls::Datagram`, so that
/// a DTLS session (`mbedtls::dtls::Session`) can run over an `edge-nal` UDP socket.
///
/// The socket can be bound or connected. The adaptor sends each datagram to the peer. It drops
/// the datagrams that come from other addresses, and the empty ones, which would end the session.
///
/// The DTLS session drops the socket's `receive` and `send` futures when they are not ready, so
/// these must not lose a datagram when dropped. The sockets of `edge-nal-std`, `edge-nal-embassy`
/// and `edge-nal-openthread` do not.
pub struct UdpDatagram<S> {
    socket: S,
    peer: SocketAddr,
}

impl<S> UdpDatagram<S> {
    /// Create a new instance of the `UdpDatagram` type.
    ///
    /// # Arguments:
    /// - `socket` - The underlying UDP socket
    /// - `peer` - The address of the DTLS peer
    pub const fn new(socket: S, peer: SocketAddr) -> Self {
        Self { socket, peer }
    }
}

impl<S> ErrorType for UdpDatagram<S>
where
    S: ErrorType,
{
    type Error = S::Error;
}

impl<S> Datagram for UdpDatagram<S>
where
    S: UdpReceive + UdpSend + Readable,
{
    fn readable(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.socket.readable()
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        loop {
            let (len, from) = self.socket.receive(buf).await?;

            // Not the whole address: a stack might leave out the scope ID
            if from.ip() == self.peer.ip() && from.port() == self.peer.port() && len > 0 {
                // For a truncated datagram, `receive` returns its full length
                break Ok(len.min(buf.len()));
            }

            debug!("Dropping a datagram of {} bytes from {}", len, from);
        }
    }

    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), Self::Error>> {
        self.socket.send(self.peer, data)
    }
}
