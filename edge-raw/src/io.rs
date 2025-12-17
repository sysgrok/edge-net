use core::fmt::Debug;
use core::mem::MaybeUninit;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use embedded_io_async::{ErrorKind, ErrorType};

use edge_nal::{MacAddr, RawReceive, RawSend, RawSplit, Readable, UdpReceive, UdpSend, UdpSplit};

use crate as raw;

/// An error that can occur when sending or receiving UDP packets over a raw socket.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Error<E> {
    Io(E),
    UnsupportedProtocol,
    RawError(raw::Error),
}

impl<E> From<raw::Error> for Error<E> {
    fn from(value: raw::Error) -> Self {
        Self::RawError(value)
    }
}

#[cfg(not(feature = "std"))]
impl<E: embedded_io_async::Error> core::error::Error for Error<E> {}

impl<E> embedded_io_async::Error for Error<E>
where
    E: embedded_io_async::Error,
{
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io(err) => err.kind(),
            Self::UnsupportedProtocol => ErrorKind::InvalidInput,
            Self::RawError(_) => ErrorKind::InvalidData,
        }
    }
}

impl<E> core::fmt::Display for Error<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "IO error: {}", err),
            Self::UnsupportedProtocol => write!(f, "Unsupported protocol"),
            Self::RawError(err) => write!(f, "Raw error: {}", err),
        }
    }
}

#[cfg(feature = "defmt")]
impl<E> defmt::Format for Error<E>
where
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Io(err) => defmt::write!(f, "IO error: {}", err),
            Self::UnsupportedProtocol => defmt::write!(f, "Unsupported protocol"),
            Self::RawError(err) => defmt::write!(f, "Raw error: {}", err),
        }
    }
}

#[cfg(feature = "std")]
impl<E> std::error::Error for Error<E> where E: std::error::Error {}

/// A utility struct allowing to send and receive UDP packets over a raw socket.
///
/// The major difference between this struct and a regular `UdpSend` and `UdpReceive` pair of UDP socket halves
/// is that `RawSocket2Udp` requires the MAC address of the remote host to send packets to.
///
/// This allows DHCP clients to operate even when the local peer does not yet have a valid IP address.
/// It also allows DHCP servers to send packets to specific clients which don't yet have an IP address, and are
/// thus only addressable either by broadcasting, or by their MAC address.
pub struct RawSocket2Udp<T, const N: usize = 1500> {
    socket: T,
    filter_local: Option<SocketAddrV4>,
    filter_remote: Option<SocketAddrV4>,
    remote_mac: MacAddr,
}

impl<T, const N: usize> RawSocket2Udp<T, N> {
    pub fn new(
        socket: T,
        filter_local: Option<SocketAddrV4>,
        filter_remote: Option<SocketAddrV4>,
        remote_mac: MacAddr,
    ) -> Self {
        Self {
            socket,
            filter_local,
            filter_remote,
            remote_mac,
        }
    }
}

impl<T, const N: usize> ErrorType for RawSocket2Udp<T, N>
where
    T: ErrorType,
{
    type Error = Error<T::Error>;
}

impl<T, const N: usize> UdpReceive for RawSocket2Udp<T, N>
where
    T: RawReceive,
{
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), Self::Error> {
        let (len, _local, remote, _) = udp_receive::<_, N>(
            &mut self.socket,
            self.filter_local,
            self.filter_remote,
            buffer,
        )
        .await?;

        Ok((len, remote))
    }
}

impl<T, const N: usize> Readable for RawSocket2Udp<T, N>
where
    T: Readable,
{
    async fn readable(&mut self) -> Result<(), Self::Error> {
        self.socket.readable().await.map_err(Error::Io)
    }
}

impl<T, const N: usize> UdpSend for RawSocket2Udp<T, N>
where
    T: RawSend,
{
    async fn send(&mut self, remote: SocketAddr, data: &[u8]) -> Result<(), Self::Error> {
        let remote = match remote {
            SocketAddr::V4(remote) => remote,
            SocketAddr::V6(_) => Err(Error::UnsupportedProtocol)?,
        };

        udp_send::<_, N>(
            &mut self.socket,
            SocketAddr::V4(
                self.filter_local
                    .unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            ),
            SocketAddr::V4(remote),
            self.remote_mac,
            data,
        )
        .await
    }
}

impl<T, const N: usize> UdpSplit for RawSocket2Udp<T, N>
where
    T: RawSplit,
{
    type Receive<'a>
        = RawSocket2Udp<T::Receive<'a>, N>
    where
        Self: 'a;
    type Send<'a>
        = RawSocket2Udp<T::Send<'a>, N>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::Receive<'_>, Self::Send<'_>) {
        let (receive, send) = self.socket.split();

        (
            RawSocket2Udp::new(
                receive,
                self.filter_local,
                self.filter_remote,
                self.remote_mac,
            ),
            RawSocket2Udp::new(send, self.filter_local, self.filter_remote, self.remote_mac),
        )
    }
}

/// Sends a UDP packet to a remote peer identified by its MAC address
pub async fn udp_send<T: RawSend, const N: usize>(
    mut socket: T,
    local: SocketAddr,
    remote: SocketAddr,
    remote_mac: MacAddr,
    data: &[u8],
) -> Result<(), Error<T::Error>> {
    let (SocketAddr::V4(local), SocketAddr::V4(remote)) = (local, remote) else {
        Err(Error::UnsupportedProtocol)?
    };

    let mut buf = MaybeUninit::<[u8; N]>::uninit();
    let buf = unsafe { buf.assume_init_mut() };

    let data = raw::ip_udp_encode(buf, local, remote, |buf| {
        if data.len() <= buf.len() {
            buf[..data.len()].copy_from_slice(data);

            Ok(data.len())
        } else {
            Err(raw::Error::BufferOverflow)
        }
    })?;

    socket.send(remote_mac, data).await.map_err(Error::Io)
}

/// Receives a UDP packet from a remote peer
pub async fn udp_receive<T: RawReceive, const N: usize>(
    mut socket: T,
    filter_local: Option<SocketAddrV4>,
    filter_remote: Option<SocketAddrV4>,
    buffer: &mut [u8],
) -> Result<(usize, SocketAddr, SocketAddr, MacAddr), Error<T::Error>> {
    let mut buf = MaybeUninit::<[u8; N]>::uninit();
    let buf = unsafe { buf.assume_init_mut() };

    let (len, local, remote, remote_mac) = loop {
        let (len, remote_mac) = socket.receive(buf).await.map_err(Error::Io)?;

        match raw::ip_udp_decode(&buf[..len], filter_remote, filter_local) {
            Ok(Some((remote, local, data))) => {
                if data.len() > buffer.len() {
                    Err(Error::RawError(raw::Error::BufferOverflow))?;
                }

                buffer[..data.len()].copy_from_slice(data);

                break (data.len(), local, remote, remote_mac);
            }
            Ok(None) => continue,
            Err(raw::Error::InvalidFormat) | Err(raw::Error::InvalidChecksum) => continue,
            Err(other) => Err(other)?,
        }
    };

    Ok((
        len,
        SocketAddr::V4(local),
        SocketAddr::V4(remote),
        remote_mac,
    ))
}
