use rand_core::Rng;

use super::*;

/// A simple DHCP client.
/// The client is unaware of the IP/UDP transport layer and operates purely in terms of packets
/// represented as Rust slices.
///
/// As such, the client can generate all BOOTP requests and parse BOOTP replies.
pub struct Client<T> {
    pub rng: T,
    pub mac: [u8; 6],
}

impl<T> Client<T>
where
    T: Rng,
{
    pub const fn new(rng: T, mac: [u8; 6]) -> Self {
        Self { rng, mac }
    }

    pub fn discover<'o>(
        &mut self,
        opt_buf: &'o mut [DhcpOption<'o>],
        secs: u16,
        ip: Option<Ipv4Addr>,
    ) -> (Packet<'o>, u32) {
        self.bootp_request(secs, None, true, Options::discover(ip, opt_buf))
    }

    pub fn request<'o>(
        &mut self,
        opt_buf: &'o mut [DhcpOption<'o>],
        secs: u16,
        ip: Ipv4Addr,
        broadcast: bool,
    ) -> (Packet<'o>, u32) {
        self.bootp_request(secs, None, broadcast, Options::request(ip, opt_buf))
    }

    pub fn release<'o>(
        &mut self,
        opt_buf: &'o mut [DhcpOption<'o>],
        secs: u16,
        ip: Ipv4Addr,
    ) -> Packet<'o> {
        self.bootp_request(secs, Some(ip), false, Options::release(opt_buf))
            .0
    }

    pub fn decline<'o>(
        &mut self,
        opt_buf: &'o mut [DhcpOption<'o>],
        secs: u16,
        ip: Ipv4Addr,
    ) -> Packet<'o> {
        self.bootp_request(secs, Some(ip), false, Options::decline(opt_buf))
            .0
    }

    pub fn is_offer(&self, reply: &Packet<'_>, xid: u32) -> bool {
        self.is_bootp_reply_for_us(reply, xid, Some(&[MessageType::Offer]))
    }

    pub fn is_ack(&self, reply: &Packet<'_>, xid: u32) -> bool {
        self.is_bootp_reply_for_us(reply, xid, Some(&[MessageType::Ack]))
    }

    pub fn is_nak(&self, reply: &Packet<'_>, xid: u32) -> bool {
        self.is_bootp_reply_for_us(reply, xid, Some(&[MessageType::Nak]))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn bootp_request<'o>(
        &mut self,
        secs: u16,
        ip: Option<Ipv4Addr>,
        broadcast: bool,
        options: Options<'o>,
    ) -> (Packet<'o>, u32) {
        let xid = self.rng.next_u32();

        (
            Packet::new_request(self.mac, xid, secs, ip, broadcast, options),
            xid,
        )
    }

    pub fn is_bootp_reply_for_us(
        &self,
        reply: &Packet<'_>,
        xid: u32,
        expected_message_types: Option<&[MessageType]>,
    ) -> bool {
        if reply.reply && reply.is_for_us(&self.mac, xid) {
            if let Some(expected_message_types) = expected_message_types {
                let mt = reply.options.iter().find_map(|option| {
                    if let DhcpOption::MessageType(mt) = option {
                        Some(mt)
                    } else {
                        None
                    }
                });

                expected_message_types.iter().any(|emt| mt == Some(*emt))
            } else {
                true
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod test {
    use core::net::Ipv4Addr;

    use rand_core::{Infallible, TryRng};

    use crate::{DhcpOption, MessageType, Options};

    use super::Client;

    /// A deterministic "random" number generator handing out consecutive numbers
    struct Counter(u32);

    impl TryRng for Counter {
        type Error = Infallible;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            self.0 += 1;
            Ok(self.0)
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            Ok(self.try_next_u32()? as u64)
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Self::Error> {
            for byte in dest {
                *byte = self.try_next_u32()? as u8;
            }
            Ok(())
        }
    }

    const MAC: [u8; 6] = [1, 2, 3, 4, 5, 6];
    const IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);

    #[test]
    fn requests() {
        let mut client = Client::new(Counter(0), MAC);

        let mut buf = Options::buf();
        let (discover, xid) = client.discover(&mut buf, 3, None);
        assert_eq!(xid, 1);
        assert_eq!(discover.xid, 1);
        assert_eq!(discover.secs, 3);
        assert!(discover.broadcast);
        assert!(!discover.reply);
        assert_eq!(&discover.chaddr[..6], &MAC);
        assert!(discover.ciaddr.is_unspecified());
        assert!(discover
            .options
            .iter()
            .eq([DhcpOption::MessageType(MessageType::Discover)]));

        let mut buf = Options::buf();
        let (request, xid) = client.request(&mut buf, 0, IP, false);
        assert_eq!(xid, 2);
        assert!(!request.broadcast);
        assert!(request.options.iter().eq([
            DhcpOption::MessageType(MessageType::Request),
            DhcpOption::RequestedIpAddress(IP),
            DhcpOption::ParameterRequestList(&[
                DhcpOption::CODE_ROUTER,
                DhcpOption::CODE_SUBNET,
                DhcpOption::CODE_DNS
            ]),
        ]));

        let mut buf = Options::buf();
        let release = client.release(&mut buf, 0, IP);
        assert_eq!(release.ciaddr, IP);
        assert!(release
            .options
            .iter()
            .eq([DhcpOption::MessageType(MessageType::Release)]));

        let mut buf = Options::buf();
        let decline = client.decline(&mut buf, 0, IP);
        assert_eq!(decline.ciaddr, IP);
        assert!(decline
            .options
            .iter()
            .eq([DhcpOption::MessageType(MessageType::Decline)]));
    }

    #[test]
    fn reply_matching() {
        let mut client = Client::new(Counter(0), MAC);

        let mut buf = Options::buf();
        let (discover, xid) = client.discover(&mut buf, 0, None);

        let offer_opts = [DhcpOption::MessageType(MessageType::Offer)];
        let offer = discover.new_reply(Some(IP), Options::new(&offer_opts));
        assert!(client.is_offer(&offer, xid));
        assert!(!client.is_ack(&offer, xid));
        assert!(!client.is_nak(&offer, xid));
        assert!(client.is_bootp_reply_for_us(&offer, xid, None));
        assert!(client.is_bootp_reply_for_us(
            &offer,
            xid,
            Some(&[MessageType::Ack, MessageType::Offer])
        ));

        // Wrong transaction ID, wrong MAC, or not a reply at all
        assert!(!client.is_offer(&offer, xid + 1));
        assert!(!client.is_bootp_reply_for_us(&discover, xid, None));
        let other = Client::new(Counter(0), [9; 6]);
        assert!(!other.is_offer(&offer, xid));

        let ack_opts = [DhcpOption::MessageType(MessageType::Ack)];
        let ack = discover.new_reply(Some(IP), Options::new(&ack_opts));
        assert!(client.is_ack(&ack, xid));
        assert!(!client.is_offer(&ack, xid));

        let nak_opts = [DhcpOption::MessageType(MessageType::Nak)];
        let nak = discover.new_reply(None, Options::new(&nak_opts));
        assert!(client.is_nak(&nak, xid));

        // A reply without a message type matches only when no type is expected
        let reply = discover.new_reply(Some(IP), Options::new(&[]));
        assert!(client.is_bootp_reply_for_us(&reply, xid, None));
        assert!(!client.is_offer(&reply, xid));
    }
}
