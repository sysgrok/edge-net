use core::cell::RefCell;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use core::pin::pin;

use buf::BufferAccess;

use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex;
use embassy_sync::blocking_mutex::raw::{NoopRawMutex, RawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;

use edge_nal::{MulticastV4, MulticastV6, Readable, UdpBind, UdpReceive, UdpSend};

use embassy_time::{Duration, Timer};

use super::*;

/// Socket address that binds to any IPv4-configured interface available
pub const IPV4_DEFAULT_SOCKET: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT);

/// Socket address that binds to any IPv6-configured interface available on single-stack
/// implementations and to any configured interface available on dual-stack implementations.
pub const IPV6_DEFAULT_SOCKET: SocketAddr =
    SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), PORT);

/// A quick-and-dirty socket address that binds to any interface available on dual-stack
/// implementations.
/// Don't use in production code.
pub const DEFAULT_SOCKET: SocketAddr = IPV6_DEFAULT_SOCKET;

/// The IPv4 mDNS broadcast address, as per spec.
pub const IP_BROADCAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
/// The IPv6 mDNS broadcast address, as per spec.
pub const IPV6_BROADCAST_ADDR: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x00fb);

/// The mDNS port, as per spec.
pub const PORT: u16 = 5353;

/// A wrapper for mDNS and IO errors.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum MdnsIoError<E> {
    MdnsError(MdnsError),
    NoRecvBufError,
    NoSendBufError,
    IoError(E),
}

pub type MdnsIoErrorKind = MdnsIoError<edge_nal::io::ErrorKind>;

impl<E> MdnsIoError<E>
where
    E: edge_nal::io::Error,
{
    pub fn erase(&self) -> MdnsIoError<edge_nal::io::ErrorKind> {
        match self {
            Self::MdnsError(e) => MdnsIoError::MdnsError(*e),
            Self::NoRecvBufError => MdnsIoError::NoRecvBufError,
            Self::NoSendBufError => MdnsIoError::NoSendBufError,
            Self::IoError(e) => MdnsIoError::IoError(e.kind()),
        }
    }
}

impl<E> From<MdnsError> for MdnsIoError<E> {
    fn from(err: MdnsError) -> Self {
        Self::MdnsError(err)
    }
}

impl<E> core::fmt::Display for MdnsIoError<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MdnsError(err) => write!(f, "mDNS error: {}", err),
            Self::NoRecvBufError => write!(f, "No recv buf available"),
            Self::NoSendBufError => write!(f, "No send buf available"),
            Self::IoError(err) => write!(f, "IO error: {}", err),
        }
    }
}

#[cfg(feature = "defmt")]
impl<E> defmt::Format for MdnsIoError<E>
where
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::MdnsError(err) => defmt::write!(f, "mDNS error: {}", err),
            Self::NoRecvBufError => defmt::write!(f, "No recv buf available"),
            Self::NoSendBufError => defmt::write!(f, "No send buf available"),
            Self::IoError(err) => defmt::write!(f, "IO error: {}", err),
        }
    }
}

impl<E> core::error::Error for MdnsIoError<E> where E: core::error::Error {}

/// A utility method to bind a socket suitable for mDNS, by using the provided
/// stack and address, and optionally joining the provided interfaces via multicast.
///
/// Note that mDNS is pointless without multicast, so at least one - or both - of the
/// ipv4 and ipv6 interfaces need to be provided.
pub async fn bind<S>(
    stack: &S,
    addr: SocketAddr,
    ipv4_interface: Option<Ipv4Addr>,
    ipv6_interface: Option<u32>,
) -> Result<S::Socket<'_>, MdnsIoError<S::Error>>
where
    S: UdpBind,
{
    let mut socket = stack.bind(addr).await.map_err(MdnsIoError::IoError)?;

    if let Some(v4) = ipv4_interface {
        socket
            .join_v4(IP_BROADCAST_ADDR, v4)
            .await
            .map_err(MdnsIoError::IoError)?;
    }

    if let Some(v6) = ipv6_interface {
        socket
            .join_v6(IPV6_BROADCAST_ADDR, v6)
            .await
            .map_err(MdnsIoError::IoError)?;
    }

    Ok(socket)
}

/// Represents an mDNS service that can respond to queries using the provided handler.
///
/// This structure is generic over the mDNS handler, the UDP receiver and sender, and the
/// raw mutex type.
///
/// The handler is expected to be a type that implements the `MdnsHandler` trait, which
/// allows it to handle mDNS queries and generate responses, as well as to handle mDNS
/// responses to queries which we might have issues using the `query` method.
pub struct Mdns<'a, R, S, RB, SB, C, M = NoopRawMutex>
where
    M: RawMutex,
{
    ipv4_interface: Option<Ipv4Addr>,
    ipv6_interface: Option<u32>,
    recv: Mutex<M, R>,
    send: Mutex<M, S>,
    recv_buf: RB,
    send_buf: SB,
    rand: blocking_mutex::Mutex<M, RefCell<C>>,
    broadcast_signal: &'a Signal<M, ()>,
    wait_readable: bool,
}

impl<'a, R, S, RB, SB, C, M> Mdns<'a, R, S, RB, SB, C, M>
where
    R: UdpReceive + Readable,
    S: UdpSend<Error = R::Error>,
    RB: BufferAccess<[u8]>,
    SB: BufferAccess<[u8]>,
    C: rand_core::RngCore,
    M: RawMutex,
{
    /// Creates a new mDNS service with the provided handler, interfaces, and UDP receiver and sender.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ipv4_interface: Option<Ipv4Addr>,
        ipv6_interface: Option<u32>,
        recv: R,
        send: S,
        recv_buf: RB,
        send_buf: SB,
        rand: C,
        broadcast_signal: &'a Signal<M, ()>,
    ) -> Self {
        Self {
            ipv4_interface,
            ipv6_interface,
            recv: Mutex::new(recv),
            send: Mutex::new(send),
            recv_buf,
            send_buf,
            rand: blocking_mutex::Mutex::new(RefCell::new(rand)),
            broadcast_signal,
            wait_readable: false,
        }
    }

    /// Sets whether the mDNS service should wait for the socket to be readable before reading.
    ///
    /// Setting this to `true` is only useful when the read buffer is shared with other tasks
    pub fn wait_readable(&mut self, wait_readable: bool) {
        self.wait_readable = wait_readable;
    }

    /// Runs the mDNS service, handling queries and responding to them, as well as broadcasting
    /// mDNS answers and handling responses to our own queries.
    ///
    /// All of the handling logic is expected to be implemented by the provided handler:
    /// - I.e. hanbdling responses to our own queries cannot happen, unless the supplied handler
    ///   is capable of doing that (i.e. it is a `PeerMdnsHandler`, or a chain containing it, or similar).
    /// - Ditto for handling queries coming from other peers - this can only happen if the handler
    ///   is capable of doing that. I.e., it is a `HostMdnsHandler`, or a chain containing it, or similar.
    pub async fn run<T>(&self, handler: T) -> Result<(), MdnsIoError<S::Error>>
    where
        T: MdnsHandler,
    {
        let handler = blocking_mutex::Mutex::<M, _>::new(RefCell::new(handler));

        let mut broadcast = pin!(self.broadcast(&handler));
        let mut respond = pin!(self.respond(&handler));

        let result = select(&mut broadcast, &mut respond).await;

        match result {
            Either::First(result) => result,
            Either::Second(result) => result,
        }
    }

    /// Sends a multicast query with the provided payload.
    /// It is assumed that the payload represents a valid mDNS query message.
    ///
    /// The payload is constructed via a closure, because this way we can provide to
    /// the payload-constructing closure a ready-to-use `&mut [u8]` slice, where the
    /// closure can arrange the mDNS query message (i.e. we avoid extra memory usage
    /// by constructing the mDNS query directly in the `send_buf` buffer that was supplied
    /// when the `Mdns` instance was constructed).
    pub async fn query<Q>(&self, q: Q) -> Result<(), MdnsIoError<S::Error>>
    where
        Q: FnOnce(&mut [u8]) -> Result<usize, MdnsError>,
    {
        let mut send_buf = self
            .send_buf
            .get()
            .await
            .ok_or(MdnsIoError::NoSendBufError)?;

        let mut send_guard = self.send.lock().await;
        let send = &mut *send_guard;

        let len = q(send_buf.as_mut())?;

        if len > 0 {
            self.broadcast_once(send, &send_buf.as_mut()[..len]).await?;
        }

        Ok(())
    }

    async fn broadcast<T>(
        &self,
        handler: &blocking_mutex::Mutex<M, RefCell<T>>,
    ) -> Result<(), MdnsIoError<S::Error>>
    where
        T: MdnsHandler,
    {
        loop {
            {
                let mut send_buf = self
                    .send_buf
                    .get()
                    .await
                    .ok_or(MdnsIoError::NoSendBufError)?;

                let mut send_guard = self.send.lock().await;
                let send = &mut *send_guard;

                let response = handler.lock(|handler| {
                    handler
                        .borrow_mut()
                        .handle(MdnsRequest::None, send_buf.as_mut())
                })?;

                if let MdnsResponse::Reply { data, delay } = response {
                    if delay {
                        // TODO: Not ideal, as we hold the lock during the delay
                        self.delay().await;
                    }

                    self.broadcast_once(send, data).await?;
                }
            }

            self.broadcast_signal.wait().await;
        }
    }

    async fn respond<T>(
        &self,
        handler: &blocking_mutex::Mutex<M, RefCell<T>>,
    ) -> Result<(), MdnsIoError<S::Error>>
    where
        T: MdnsHandler,
    {
        let mut recv = self.recv.lock().await;

        loop {
            if self.wait_readable {
                recv.readable().await.map_err(MdnsIoError::IoError)?;
            }

            {
                let mut recv_buf = self
                    .recv_buf
                    .get()
                    .await
                    .ok_or(MdnsIoError::NoRecvBufError)?;

                let (len, remote) = recv
                    .receive(recv_buf.as_mut())
                    .await
                    .map_err(MdnsIoError::IoError)?;

                debug!("Got mDNS query from {}", remote);

                {
                    let mut send_buf = self
                        .send_buf
                        .get()
                        .await
                        .ok_or(MdnsIoError::NoSendBufError)?;

                    let mut send_guard = self.send.lock().await;
                    let send = &mut *send_guard;

                    let response = match handler.lock(|handler| {
                        handler.borrow_mut().handle(
                            MdnsRequest::Request {
                                data: &recv_buf.as_mut()[..len],
                                legacy: remote.port() != PORT,
                                multicast: true, // TODO: Cannot determine this
                            },
                            send_buf.as_mut(),
                        )
                    }) {
                        Ok(len) => len,
                        Err(err) => match err {
                            MdnsError::InvalidMessage => {
                                warn!("Got invalid message from {}, skipping", remote);
                                continue;
                            }
                            other => Err(other)?,
                        },
                    };

                    if let MdnsResponse::Reply { data, delay } = response {
                        if remote.port() != PORT {
                            // Support one-shot legacy queries by replying privately
                            // to the remote address, if the query was not sent from the mDNS port (as per the spec)

                            debug!(
                                "Replying privately to a one-shot mDNS query from {}",
                                remote
                            );

                            if let Err(err) = send.send(remote, data).await {
                                warn!(
                                    "Failed to reply privately to {}: {:?}",
                                    remote,
                                    debug2format!(err)
                                );
                            }
                        } else {
                            // Otherwise, re-broadcast the response

                            if delay {
                                self.delay().await;
                            }

                            debug!("Re-broadcasting due to mDNS query from {}", remote);

                            self.broadcast_once(send, data).await?;
                        }
                    }
                }
            }
        }
    }

    async fn broadcast_once(&self, send: &mut S, data: &[u8]) -> Result<(), MdnsIoError<S::Error>> {
        for remote_addr in
            core::iter::once(SocketAddr::V4(SocketAddrV4::new(IP_BROADCAST_ADDR, PORT)))
                .filter(|_| self.ipv4_interface.is_some())
                .chain(
                    self.ipv6_interface
                        .map(|interface| {
                            SocketAddr::V6(SocketAddrV6::new(
                                IPV6_BROADCAST_ADDR,
                                PORT,
                                0,
                                interface,
                            ))
                        })
                        .into_iter(),
                )
        {
            if !data.is_empty() {
                debug!("Broadcasting mDNS entry to {}", remote_addr);

                let fut = pin!(send.send(remote_addr, data));

                fut.await.map_err(MdnsIoError::IoError)?;
            }
        }

        Ok(())
    }

    async fn delay(&self) {
        let mut b = [0];
        self.rand.lock(|rand| rand.borrow_mut().fill_bytes(&mut b));

        // Generate a delay between 20 and 120 ms, as per spec
        let delay_ms = 20 + (b[0] as u32 * 100 / 256);

        Timer::after(Duration::from_millis(delay_ms as _)).await;
    }
}
