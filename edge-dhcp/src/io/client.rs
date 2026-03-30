use core::fmt::Debug;
use core::net::Ipv4Addr;

use edge_nal::{UdpReceive, UdpSend};
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Instant, Timer};

use rand_core::Rng;

pub use super::*;

pub use crate::Settings;
use crate::{Options, Packet};

/// Represents the additional network-related information that might be returned by the DHCP server.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct NetworkInfo<'a> {
    pub gateway: Option<Ipv4Addr>,
    pub subnet: Option<Ipv4Addr>,
    pub dns1: Option<Ipv4Addr>,
    pub dns2: Option<Ipv4Addr>,
    pub captive_url: Option<&'a str>,
}

/// Represents a DHCP IP lease.
///
/// This structure has a set of asynchronous methods that can utilize a supplied DHCP client instance and UDP socket to
/// transparently implement all aspects of negotiating an IP with the DHCP server and then keeping the lease of that IP up to date.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Lease {
    pub ip: Ipv4Addr,
    pub server_ip: Ipv4Addr,
    pub duration: Duration,
    pub acquired: Instant,
}

impl Lease {
    /// Creates a new DHCP lease by discovering a DHCP server and requesting an IP from it.
    /// This is done by utilizing the supplied DHCP client instance and UDP socket.
    ///
    /// Note that the supplied UDP socket should be capable of sending and receiving broadcast UDP packets.
    pub async fn new<'a, T, S>(
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &'a mut [u8],
    ) -> Result<(Self, NetworkInfo<'a>), Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        loop {
            let offer = Self::discover(client, socket, buf, Duration::from_secs(3)).await?;
            let server_ip = unwrap!(offer.server_ip);
            let ip = offer.ip;

            let now = Instant::now();

            {
                // Nasty but necessary to avoid Rust's borrow checker not dealing
                // with the non-lexical lifetimes involved here
                let buf = unsafe { Self::unsafe_reborrow(buf) };

                if let Some(settings) = Self::request(
                    client,
                    socket,
                    buf,
                    server_ip,
                    ip,
                    true,
                    Duration::from_secs(3),
                    3,
                )
                .await?
                {
                    break Ok((
                        Self {
                            ip: settings.ip,
                            server_ip: unwrap!(settings.server_ip),
                            duration: Duration::from_secs(
                                settings.lease_time_secs.unwrap_or(7200) as _
                            ),
                            acquired: now,
                        },
                        NetworkInfo {
                            gateway: settings.gateway,
                            subnet: settings.subnet,
                            dns1: settings.dns1,
                            dns2: settings.dns2,
                            captive_url: settings.captive_url,
                        },
                    ));
                }
            }
        }
    }

    /// Keeps the DHCP lease up to date by renewing it when necessary using the supplied DHCP client instance and UDP socket.
    pub async fn keep<T, S>(
        &mut self,
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &mut [u8],
    ) -> Result<(), Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        loop {
            let now = Instant::now();

            if now - self.acquired >= self.duration / 3 {
                if !self.renew(client, socket, buf).await? {
                    // Lease was not renewed; let the user know
                    break;
                }
            } else {
                Timer::after(Duration::from_secs(60)).await;
            }
        }

        Ok(())
    }

    /// Renews the DHCP lease by utilizing the supplied DHCP client instance and UDP socket.
    pub async fn renew<T, S>(
        &mut self,
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &mut [u8],
    ) -> Result<bool, Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        info!("Renewing DHCP lease...");

        let now = Instant::now();
        let settings = Self::request(
            client,
            socket,
            buf,
            self.server_ip,
            self.ip,
            false,
            Duration::from_secs(3),
            3,
        )
        .await?;

        if let Some(settings) = settings.as_ref() {
            self.duration = settings
                .lease_time_secs
                .map(|lt| Duration::from_secs(lt as _))
                .unwrap_or(self.duration);
            self.acquired = now;
        }

        Ok(settings.is_some())
    }

    /// Releases the DHCP lease by utilizing the supplied DHCP client instance and UDP socket.
    pub async fn release<T, S>(
        self,
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &mut [u8],
    ) -> Result<(), Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        let mut opt_buf = Options::buf();
        let request = client.release(&mut opt_buf, 0, self.ip);

        socket
            .send(
                SocketAddr::V4(SocketAddrV4::new(self.server_ip, DEFAULT_SERVER_PORT)),
                request.encode(buf)?,
            )
            .await
            .map_err(Error::Io)?;

        Ok(())
    }

    async fn discover<'a, T, S>(
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &'a mut [u8],
        timeout: Duration,
    ) -> Result<Settings<'a>, Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        info!("Discovering DHCP servers...");

        let start = Instant::now();

        loop {
            let mut opt_buf = Options::buf();

            let (request, xid) =
                client.discover(&mut opt_buf, (Instant::now() - start).as_secs() as _, None);

            socket
                .send(
                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_SERVER_PORT)),
                    request.encode(buf)?,
                )
                .await
                .map_err(Error::Io)?;

            if let Either::First(result) = select(socket.receive(buf), Timer::after(timeout)).await
            {
                // Nasty but necessary to avoid Rust's borrow checker not dealing
                // with the non-lexical lifetimes involved here
                let buf = unsafe { Self::unsafe_reborrow(buf) };

                let (len, _remote) = result.map_err(Error::Io)?;
                let reply = Packet::decode(&buf[..len])?;

                if client.is_offer(&reply, xid) {
                    let settings = Settings::new(&reply);

                    info!(
                        "IP {} offered by DHCP server {}",
                        settings.ip,
                        unwrap!(settings.server_ip)
                    );

                    return Ok(settings);
                }
            }

            info!("No DHCP offers received, retrying...");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn request<'a, T, S>(
        client: &mut dhcp::client::Client<T>,
        socket: &mut S,
        buf: &'a mut [u8],
        server_ip: Ipv4Addr,
        ip: Ipv4Addr,
        broadcast: bool,
        timeout: Duration,
        retries: usize,
    ) -> Result<Option<Settings<'a>>, Error<S::Error>>
    where
        T: Rng,
        S: UdpReceive + UdpSend,
    {
        for _ in 0..retries {
            info!("Requesting IP {} from DHCP server {}", ip, server_ip);

            let start = Instant::now();

            let mut opt_buf = Options::buf();

            let (request, xid) = client.request(
                &mut opt_buf,
                (Instant::now() - start).as_secs() as _,
                ip,
                broadcast,
            );

            socket
                .send(
                    SocketAddr::V4(SocketAddrV4::new(
                        if broadcast {
                            Ipv4Addr::BROADCAST
                        } else {
                            server_ip
                        },
                        DEFAULT_SERVER_PORT,
                    )),
                    request.encode(buf)?,
                )
                .await
                .map_err(Error::Io)?;

            if let Either::First(result) = select(socket.receive(buf), Timer::after(timeout)).await
            {
                let (len, _remote) = result.map_err(Error::Io)?;

                // Nasty but necessary to avoid Rust's borrow checker not dealing
                // with the non-lexical lifetimes involved here
                let buf = unsafe { Self::unsafe_reborrow(buf) };

                let packet = &buf[..len];

                let reply = Packet::decode(packet)?;

                if client.is_ack(&reply, xid) {
                    let settings = Settings::new(&reply);

                    info!("IP {} leased successfully", ip);

                    return Ok(Some(settings));
                } else if client.is_nak(&reply, xid) {
                    info!("IP {} not acknowledged", ip);

                    return Ok(None);
                }
            }
        }

        warn!("IP request was not replied");

        Ok(None)
    }

    // Useful when Rust's borrow-checker still cannot handle some NLLs
    // https://rust-lang.github.io/rfcs/2094-nll.html
    unsafe fn unsafe_reborrow<'a>(buf: &mut [u8]) -> &'a mut [u8] {
        let len = buf.len();
        unsafe { core::slice::from_raw_parts_mut(buf.as_mut_ptr(), len) }
    }
}
