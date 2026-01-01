use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;

use edge_nal::{UdpBind, UdpReceive, UdpSend};

use super::*;

pub const DEFAULT_SOCKET: SocketAddr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), PORT);

const PORT: u16 = 53;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DnsIoError<E> {
    DnsError(DnsError),
    IoError(E),
}

pub type DnsIoErrorKind = DnsIoError<edge_nal::io::ErrorKind>;

impl<E> DnsIoError<E>
where
    E: edge_nal::io::Error,
{
    pub fn erase(&self) -> DnsIoError<edge_nal::io::ErrorKind> {
        match self {
            Self::DnsError(e) => DnsIoError::DnsError(*e),
            Self::IoError(e) => DnsIoError::IoError(e.kind()),
        }
    }
}

impl<E> From<DnsError> for DnsIoError<E> {
    fn from(err: DnsError) -> Self {
        Self::DnsError(err)
    }
}

impl<E> fmt::Display for DnsIoError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DnsError(err) => write!(f, "DNS error: {}", err),
            Self::IoError(err) => write!(f, "IO error: {}", err),
        }
    }
}

#[cfg(feature = "defmt")]
impl<E> defmt::Format for DnsIoError<E>
where
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::DnsError(err) => defmt::write!(f, "DNS error: {}", err),
            Self::IoError(err) => defmt::write!(f, "IO error: {}", err),
        }
    }
}

impl<E> core::error::Error for DnsIoError<E> where E: core::error::Error {}

pub async fn run<S>(
    stack: &S,
    local_addr: SocketAddr,
    tx_buf: &mut [u8],
    rx_buf: &mut [u8],
    ip: Ipv4Addr,
    ttl: Duration,
) -> Result<(), DnsIoError<S::Error>>
where
    S: UdpBind,
{
    let mut udp = stack.bind(local_addr).await.map_err(DnsIoError::IoError)?;

    loop {
        debug!("Waiting for data");

        let (len, remote) = udp.receive(rx_buf).await.map_err(DnsIoError::IoError)?;

        let request = &rx_buf[..len];

        debug!("Received {} bytes from {}", request.len(), remote);

        let len = match crate::reply(request, &ip.octets(), ttl, tx_buf) {
            Ok(len) => len,
            Err(err) => match err {
                DnsError::InvalidMessage => {
                    warn!("Got invalid message from {}, skipping", remote);
                    continue;
                }
                other => Err(other)?,
            },
        };

        udp.send(remote, &tx_buf[..len])
            .await
            .map_err(DnsIoError::IoError)?;

        debug!("Sent {} bytes to {}", len, remote);
    }
}
