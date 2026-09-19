use core::fmt::Debug;

use super::*;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Lease {
    mac: [u8; 16],
    expires: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Action<'a> {
    Discover(Option<Ipv4Addr>, &'a [u8; 16]),
    Request(Ipv4Addr, &'a [u8; 16]),
    Release(Ipv4Addr, &'a [u8; 16]),
    Decline(Ipv4Addr, &'a [u8; 16]),
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct ServerOptions<'a> {
    pub ip: Ipv4Addr,
    pub gateways: &'a [Ipv4Addr],
    pub subnet: Option<Ipv4Addr>,
    pub dns: &'a [Ipv4Addr],
    pub captive_url: Option<&'a str>,
    pub lease_duration_secs: u32,
}

impl<'a> ServerOptions<'a> {
    pub fn new(ip: Ipv4Addr, gw_buf: Option<&'a mut [Ipv4Addr; 1]>) -> Self {
        let gateways = if let Some(gw_buf) = gw_buf {
            gw_buf[0] = ip;
            gw_buf.as_slice()
        } else {
            &[]
        };

        Self {
            ip,
            gateways,
            subnet: Some(Ipv4Addr::new(255, 255, 255, 0)),
            dns: &[],
            captive_url: None,
            lease_duration_secs: 7200,
        }
    }

    pub fn process<'o>(&self, request: &'o Packet<'o>) -> Option<Action<'o>> {
        if request.reply {
            return None;
        }

        let message_type = request.options.iter().find_map(|option| {
            if let DhcpOption::MessageType(message_type) = option {
                Some(message_type)
            } else {
                None
            }
        });

        let message_type = if let Some(message_type) = message_type {
            message_type
        } else {
            warn!(
                "Ignoring DHCP request, no message type found: {:?}",
                request
            );
            return None;
        };

        let server_identifier = request.options.iter().find_map(|option| {
            if let DhcpOption::ServerIdentifier(ip) = option {
                Some(ip)
            } else {
                None
            }
        });

        if server_identifier.is_some() && server_identifier != Some(self.ip) {
            warn!(
                "Ignoring {} request, not addressed to this server: {:?}",
                message_type, request
            );
            return None;
        }

        debug!("Received {} request: {:?}", message_type, request);
        match message_type {
            MessageType::Discover => Some(Action::Discover(
                request.options.requested_ip(),
                &request.chaddr,
            )),
            MessageType::Request => {
                let requested_ip = request.options.requested_ip().or_else(|| {
                    if request.ciaddr.is_unspecified() {
                        None
                    } else {
                        Some(request.ciaddr)
                    }
                })?;

                Some(Action::Request(requested_ip, &request.chaddr))
            }
            MessageType::Release if server_identifier == Some(self.ip) => {
                Some(Action::Release(request.yiaddr, &request.chaddr))
            }
            MessageType::Decline if server_identifier == Some(self.ip) => {
                Some(Action::Decline(request.yiaddr, &request.chaddr))
            }
            _ => None,
        }
    }

    pub fn offer(
        &self,
        request: &Packet,
        yiaddr: Ipv4Addr,
        opt_buf: &'a mut [DhcpOption<'a>],
    ) -> Packet<'a> {
        self.reply(request, MessageType::Offer, Some(yiaddr), opt_buf)
    }

    pub fn ack_nak(
        &self,
        request: &Packet,
        ip: Option<Ipv4Addr>,
        opt_buf: &'a mut [DhcpOption<'a>],
    ) -> Packet<'a> {
        self.reply(
            request,
            if ip.is_some() {
                MessageType::Ack
            } else {
                MessageType::Nak
            },
            ip,
            opt_buf,
        )
    }

    fn reply(
        &self,
        request: &Packet,
        message_type: MessageType,
        ip: Option<Ipv4Addr>,
        buf: &'a mut [DhcpOption<'a>],
    ) -> Packet<'a> {
        let reply = request.new_reply(
            ip,
            request.options.reply(
                message_type,
                self.ip,
                self.lease_duration_secs as _,
                self.gateways,
                self.subnet,
                self.dns,
                self.captive_url,
                buf,
            ),
        );

        debug!("Sending {} reply: {:?}", message_type, reply);

        reply
    }
}

/// A simple DHCP server.
/// The server is unaware of the IP/UDP transport layer and operates purely in terms of packets
/// represented as Rust slices.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Server<F, const N: usize> {
    pub now: F,
    pub range_start: Ipv4Addr,
    pub range_end: Ipv4Addr,
    pub leases: heapless::LinearMap<Ipv4Addr, Lease, N>,
}

impl<F, const N: usize> Server<F, N>
where
    F: FnMut() -> u64,
{
    /// Create a new DHCP server.
    ///
    /// # Arguments
    /// - `now`: A closure that returns the current time in seconds since some epoch.
    /// - `ip`: The IP address of the server.
    pub const fn new(now: F, ip: Ipv4Addr) -> Self {
        let octets = ip.octets();

        Self {
            now,
            range_start: Ipv4Addr::new(octets[0], octets[1], octets[2], 50),
            range_end: Ipv4Addr::new(octets[0], octets[1], octets[2], 200),
            leases: heapless::LinearMap::new(),
        }
    }

    pub fn handle_request<'o>(
        &mut self,
        opt_buf: &'o mut [DhcpOption<'o>],
        server_options: &'o ServerOptions,
        request: &Packet,
    ) -> Option<Packet<'o>> {
        server_options
            .process(request)
            .and_then(|action| match action {
                Action::Discover(requested_ip, mac) => {
                    let ip = requested_ip
                        .and_then(|ip| self.is_available(mac, ip).then_some(ip))
                        .or_else(|| self.current_lease(mac))
                        .or_else(|| self.available());

                    ip.map(|ip| server_options.offer(request, ip, opt_buf))
                }
                Action::Request(ip, mac) => {
                    let now = (self.now)();

                    let ip = (self.is_available(mac, ip)
                        && self.add_lease(
                            ip,
                            request.chaddr,
                            now + server_options.lease_duration_secs as u64,
                        ))
                    .then_some(ip);

                    Some(server_options.ack_nak(request, ip, opt_buf))
                }
                Action::Release(_ip, mac) | Action::Decline(_ip, mac) => {
                    self.remove_lease(mac);

                    None
                }
            })
    }

    fn is_available(&mut self, mac: &[u8; 16], addr: Ipv4Addr) -> bool {
        let pos: u32 = addr.into();

        let start: u32 = self.range_start.into();
        let end: u32 = self.range_end.into();

        pos >= start
            && pos <= end
            && match self.leases.get(&addr) {
                Some(lease) => lease.mac == *mac || (self.now)() > lease.expires,
                None => true,
            }
    }

    fn available(&mut self) -> Option<Ipv4Addr> {
        let start: u32 = self.range_start.into();
        let end: u32 = self.range_end.into();

        for pos in start..end + 1 {
            let addr = pos.into();

            if !self.leases.contains_key(&addr) {
                return Some(addr);
            }
        }

        if let Some(addr) = self
            .leases
            .iter()
            .find_map(|(addr, lease)| ((self.now)() > lease.expires).then_some(*addr))
        {
            self.leases.remove(&addr);

            Some(addr)
        } else {
            None
        }
    }

    fn current_lease(&self, mac: &[u8; 16]) -> Option<Ipv4Addr> {
        self.leases
            .iter()
            .find_map(|(addr, lease)| (lease.mac == *mac).then_some(*addr))
    }

    fn add_lease(&mut self, addr: Ipv4Addr, mac: [u8; 16], expires: u64) -> bool {
        self.remove_lease(&mac);

        self.leases.insert(addr, Lease { mac, expires }).is_ok()
    }

    fn remove_lease(&mut self, mac: &[u8; 16]) -> bool {
        if let Some(addr) = self.current_lease(mac) {
            self.leases.remove(&addr);

            true
        } else {
            false
        }
    }
}

#[cfg(feature = "io")]
impl<const N: usize> Server<fn() -> u64, N> {
    /// Create a new DHCP server using `embassy-time::Instant::now` as the currtent time epoch provider.
    ///
    /// # Arguments
    /// - `ip`: The IP address of the server.
    pub const fn new_with_et(ip: Ipv4Addr) -> Self {
        Self::new(|| embassy_time::Instant::now().as_secs(), ip)
    }
}

#[cfg(test)]
mod test {
    use core::cell::Cell;
    use core::net::Ipv4Addr;

    use crate::{DhcpOption, MessageType, Options, Packet, Settings};

    use super::{Server, ServerOptions};

    const SERVER_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 0, 1);

    fn mac(n: u8) -> [u8; 6] {
        [0x02, 0, 0, 0, 0, n]
    }

    fn message_type(packet: &Packet) -> Option<MessageType> {
        packet.options.iter().find_map(|option| match option {
            DhcpOption::MessageType(mt) => Some(mt),
            _ => None,
        })
    }

    #[test]
    fn dora_and_leases() {
        let clock = Cell::new(1000_u64);
        let mut server = Server::<_, 4>::new(|| clock.get(), SERVER_IP);
        let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
        let server_options = ServerOptions::new(SERVER_IP, Some(&mut gw_buf));

        // Discover: the first address of the range is offered
        let mut opt_buf = Options::buf();
        let discover = Packet::new_request(
            mac(1),
            11,
            0,
            None,
            true,
            Options::discover(None, &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let offer = server
            .handle_request(&mut reply_buf, &server_options, &discover)
            .unwrap();
        assert_eq!(message_type(&offer), Some(MessageType::Offer));
        assert!(offer.reply);
        assert_eq!(offer.xid, 11);
        assert_eq!(offer.chaddr, discover.chaddr);
        assert_eq!(offer.yiaddr, Ipv4Addr::new(192, 168, 0, 50));
        assert!(server.leases.is_empty());

        // Request: the address is acknowledged and leased, with the requested parameters filled in
        let mut opt_buf = Options::buf();
        let request = Packet::new_request(
            mac(1),
            12,
            0,
            None,
            true,
            Options::request(offer.yiaddr, &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let ack = server
            .handle_request(&mut reply_buf, &server_options, &request)
            .unwrap();
        assert_eq!(message_type(&ack), Some(MessageType::Ack));
        assert_eq!(ack.yiaddr, Ipv4Addr::new(192, 168, 0, 50));
        let settings = Settings::new(&ack);
        assert_eq!(settings.server_ip, Some(SERVER_IP));
        assert_eq!(settings.lease_time_secs, Some(7200));
        assert_eq!(settings.gateway, Some(SERVER_IP));
        assert_eq!(settings.subnet, Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(settings.dns1, None);
        assert_eq!(server.leases.len(), 1);

        // A second client is offered the next free address
        let mut opt_buf = Options::buf();
        let discover2 = Packet::new_request(
            mac(2),
            21,
            0,
            None,
            true,
            Options::discover(None, &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let offer2 = server
            .handle_request(&mut reply_buf, &server_options, &discover2)
            .unwrap();
        assert_eq!(offer2.yiaddr, Ipv4Addr::new(192, 168, 0, 51));

        // The first client re-discovering gets its own lease back
        let mut reply_buf = Options::buf();
        let offer = server
            .handle_request(&mut reply_buf, &server_options, &discover)
            .unwrap();
        assert_eq!(offer.yiaddr, Ipv4Addr::new(192, 168, 0, 50));

        // Requesting an address leased to somebody else is refused
        let mut opt_buf = Options::buf();
        let request2 = Packet::new_request(
            mac(2),
            22,
            0,
            None,
            true,
            Options::request(Ipv4Addr::new(192, 168, 0, 50), &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let nak = server
            .handle_request(&mut reply_buf, &server_options, &request2)
            .unwrap();
        assert_eq!(message_type(&nak), Some(MessageType::Nak));
        assert!(nak.yiaddr.is_unspecified());
        assert_eq!(server.leases.len(), 1);

        // Release frees the lease
        let release_opts = [
            DhcpOption::MessageType(MessageType::Release),
            DhcpOption::ServerIdentifier(SERVER_IP),
        ];
        let release = Packet::new_request(
            mac(1),
            13,
            0,
            Some(Ipv4Addr::new(192, 168, 0, 50)),
            false,
            Options::new(&release_opts),
        );
        let mut reply_buf = Options::buf();
        assert!(server
            .handle_request(&mut reply_buf, &server_options, &release)
            .is_none());
        assert!(server.leases.is_empty());

        // A leased address is refused to others until the lease expires
        let mut opt_buf = Options::buf();
        let request3 = Packet::new_request(
            mac(3),
            31,
            0,
            None,
            true,
            Options::request(Ipv4Addr::new(192, 168, 0, 60), &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let ack3 = server
            .handle_request(&mut reply_buf, &server_options, &request3)
            .unwrap();
        assert_eq!(message_type(&ack3), Some(MessageType::Ack));

        let mut opt_buf = Options::buf();
        let request4 = Packet::new_request(
            mac(4),
            41,
            0,
            None,
            true,
            Options::request(Ipv4Addr::new(192, 168, 0, 60), &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let nak4 = server
            .handle_request(&mut reply_buf, &server_options, &request4)
            .unwrap();
        assert_eq!(message_type(&nak4), Some(MessageType::Nak));

        clock.set(1000 + 7201);
        let mut reply_buf = Options::buf();
        let ack4 = server
            .handle_request(&mut reply_buf, &server_options, &request4)
            .unwrap();
        assert_eq!(message_type(&ack4), Some(MessageType::Ack));
        assert_eq!(ack4.yiaddr, Ipv4Addr::new(192, 168, 0, 60));
    }

    #[test]
    fn ignored_requests() {
        let mut server = Server::<_, 4>::new(|| 0_u64, SERVER_IP);
        let server_options = ServerOptions::new(SERVER_IP, None);
        let ip = Ipv4Addr::new(192, 168, 0, 50);

        // Addressed to another server
        let opts = [
            DhcpOption::MessageType(MessageType::Request),
            DhcpOption::ServerIdentifier(Ipv4Addr::new(192, 168, 0, 2)),
            DhcpOption::RequestedIpAddress(ip),
        ];
        let request = Packet::new_request(mac(1), 1, 0, None, true, Options::new(&opts));
        let mut reply_buf = Options::buf();
        assert!(server
            .handle_request(&mut reply_buf, &server_options, &request)
            .is_none());

        // Without a message type
        let request = Packet::new_request(mac(1), 2, 0, None, true, Options::new(&[]));
        let mut reply_buf = Options::buf();
        assert!(server
            .handle_request(&mut reply_buf, &server_options, &request)
            .is_none());

        // A `Request` without a requested address
        let opts = [DhcpOption::MessageType(MessageType::Request)];
        let request = Packet::new_request(mac(1), 3, 0, None, true, Options::new(&opts));
        let mut reply_buf = Options::buf();
        assert!(server
            .handle_request(&mut reply_buf, &server_options, &request)
            .is_none());

        // Replies are never processed
        let opts = [DhcpOption::MessageType(MessageType::Discover)];
        let reply = Packet::new_request(mac(1), 4, 0, None, true, Options::new(&opts))
            .new_reply(None, Options::new(&opts));
        let mut reply_buf = Options::buf();
        assert!(server
            .handle_request(&mut reply_buf, &server_options, &reply)
            .is_none());

        assert!(server.leases.is_empty());
    }

    #[test]
    fn lease_table_capacity() {
        let mut server = Server::<_, 1>::new(|| 0_u64, SERVER_IP);
        let server_options = ServerOptions::new(SERVER_IP, None);

        let mut opt_buf = Options::buf();
        let request = Packet::new_request(
            mac(1),
            1,
            0,
            None,
            true,
            Options::request(Ipv4Addr::new(192, 168, 0, 50), &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let ack = server
            .handle_request(&mut reply_buf, &server_options, &request)
            .unwrap();
        assert_eq!(message_type(&ack), Some(MessageType::Ack));

        // The lease table is full: a second client gets a NAK
        let mut opt_buf = Options::buf();
        let request = Packet::new_request(
            mac(2),
            2,
            0,
            None,
            true,
            Options::request(Ipv4Addr::new(192, 168, 0, 51), &mut opt_buf),
        );
        let mut reply_buf = Options::buf();
        let nak = server
            .handle_request(&mut reply_buf, &server_options, &request)
            .unwrap();
        assert_eq!(message_type(&nak), Some(MessageType::Nak));
        assert_eq!(server.leases.len(), 1);
    }
}
