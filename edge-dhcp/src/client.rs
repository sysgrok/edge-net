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
