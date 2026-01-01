#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

/// This code is a `no_std` and no-alloc modification of https://github.com/krolaw/dhcp4r
use core::str::Utf8Error;

pub use core::net::Ipv4Addr;

use num_enum::TryFromPrimitive;

use edge_raw::bytes::{self, BytesIn, BytesOut};

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

pub mod client;
pub mod server;

#[cfg(feature = "io")]
pub mod io;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    DataUnderflow,
    BufferOverflow,
    InvalidPacket,
    InvalidUtf8Str(Utf8Error),
    InvalidMessageType,
    MissingCookie,
    InvalidHlen,
}

impl From<bytes::Error> for Error {
    fn from(value: bytes::Error) -> Self {
        match value {
            bytes::Error::BufferOverflow => Self::BufferOverflow,
            bytes::Error::DataUnderflow => Self::DataUnderflow,
            bytes::Error::InvalidFormat => Self::InvalidPacket,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let str = match self {
            Self::DataUnderflow => "Data underflow",
            Self::BufferOverflow => "Buffer overflow",
            Self::InvalidPacket => "Invalid packet",
            Self::InvalidUtf8Str(_) => "Invalid Utf8 string",
            Self::InvalidMessageType => "Invalid message type",
            Self::MissingCookie => "Missing cookie",
            Self::InvalidHlen => "Invalid hlen",
        };

        write!(f, "{}", str)
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Error {
    fn format(&self, f: defmt::Formatter<'_>) {
        let str = match self {
            Self::DataUnderflow => "Data underflow",
            Self::BufferOverflow => "Buffer overflow",
            Self::InvalidPacket => "Invalid packet",
            Self::InvalidUtf8Str(_) => "Invalid Utf8 string",
            Self::InvalidMessageType => "Invalid message type",
            Self::MissingCookie => "Missing cookie",
            Self::InvalidHlen => "Invalid hlen",
        };

        defmt::write!(f, "{}", str)
    }
}

impl core::error::Error for Error {}

///
/// DHCP Message Type.
///
/// # Standards
///
/// The semantics of the various DHCP message types are described in RFC 2131 (see Table 2).
/// Their numeric values are described in Section 9.6 of RFC 2132, which begins:
///
/// > This option is used to convey the type of the DHCP message.  The code for this option is 53,
/// > and its length is 1.
///
#[derive(Copy, Clone, PartialEq, Eq, Debug, TryFromPrimitive)]
#[repr(u8)]
pub enum MessageType {
    /// Client broadcast to locate available servers.
    Discover = 1,

    /// Server to client in response to DHCPDISCOVER with offer of configuration parameters.
    Offer = 2,

    /// Client message to servers either (a) requesting offered parameters from one server and
    /// implicitly declining offers from all others, (b) confirming correctness of previously
    /// allocated address after, e.g., system reboot, or (c) extending the lease on a particular
    /// network address.
    Request = 3,

    /// Client to server indicating network address is already in use.
    Decline = 4,

    /// Server to client with configuration parameters, including committed network address.
    Ack = 5,

    /// Server to client indicating client's notion of network address is incorrect (e.g., client
    /// has moved to new subnet) or client's lease as expired.
    Nak = 6,

    /// Client to server relinquishing network address and cancelling remaining lease.
    Release = 7,

    /// Client to server, asking only for local configuration parameters; client already has
    /// externally configured network address.
    Inform = 8,
}

impl core::fmt::Display for MessageType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Discover => "DHCPDISCOVER",
            Self::Offer => "DHCPOFFER",
            Self::Request => "DHCPREQUEST",
            Self::Decline => "DHCPDECLINE",
            Self::Ack => "DHCPACK",
            Self::Nak => "DHCPNAK",
            Self::Release => "DHCPRELEASE",
            Self::Inform => "DHCPINFORM",
        }
        .fmt(f)
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for MessageType {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Discover => "DHCPDISCOVER",
            Self::Offer => "DHCPOFFER",
            Self::Request => "DHCPREQUEST",
            Self::Decline => "DHCPDECLINE",
            Self::Ack => "DHCPACK",
            Self::Nak => "DHCPNAK",
            Self::Release => "DHCPRELEASE",
            Self::Inform => "DHCPINFORM",
        }
        .format(f)
    }
}

/// DHCP Packet Structure
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Packet<'a> {
    pub reply: bool,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub broadcast: bool,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: [u8; 16],
    pub options: Options<'a>,
}

impl<'a> Packet<'a> {
    const COOKIE: [u8; 4] = [99, 130, 83, 99];

    const BOOT_REQUEST: u8 = 1; // From Client
    const BOOT_REPLY: u8 = 2; // From Server

    const SERVER_NAME_AND_FILE_NAME: usize = 64 + 128;

    const END: u8 = 255;
    const PAD: u8 = 0;

    pub fn new_request(
        mac: [u8; 6],
        xid: u32,
        secs: u16,
        our_ip: Option<Ipv4Addr>,
        broadcast: bool,
        options: Options<'a>,
    ) -> Self {
        let mut chaddr = [0; 16];
        chaddr[..6].copy_from_slice(&mac);

        Self {
            reply: false,
            hops: 0,
            xid,
            secs,
            broadcast,
            ciaddr: our_ip.unwrap_or(Ipv4Addr::UNSPECIFIED),
            yiaddr: our_ip.unwrap_or(Ipv4Addr::UNSPECIFIED),
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            options,
        }
    }

    pub fn new_reply<'b>(&self, ip: Option<Ipv4Addr>, options: Options<'b>) -> Packet<'b> {
        let mut ciaddr = Ipv4Addr::UNSPECIFIED;
        if ip.is_some() {
            for opt in self.options.iter() {
                if matches!(opt, DhcpOption::MessageType(MessageType::Request)) {
                    ciaddr = self.ciaddr;
                    break;
                }
            }
        }

        Packet {
            reply: true,
            hops: 0,
            xid: self.xid,
            secs: 0,
            broadcast: self.broadcast,
            ciaddr,
            yiaddr: ip.unwrap_or(Ipv4Addr::UNSPECIFIED),
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: self.giaddr,
            chaddr: self.chaddr,
            options,
        }
    }

    pub fn is_for_us(&self, mac: &[u8; 6], xid: u32) -> bool {
        const MAC_TRAILING_ZEROS: [u8; 10] = [0; 10];

        self.chaddr[0..6] == *mac
            && self.chaddr[6..16] == MAC_TRAILING_ZEROS
            && self.xid == xid
            && self.reply
    }

    /// Parses the packet from a byte slice
    pub fn decode(data: &'a [u8]) -> Result<Self, Error> {
        let mut bytes = BytesIn::new(data);

        Ok(Self {
            reply: {
                let reply = bytes.byte()? == Self::BOOT_REPLY;
                let _htype = bytes.byte()?; // Hardware address type; 1 = 10Mb Ethernet
                let hlen = bytes.byte()?;

                if hlen != 6 {
                    Err(Error::InvalidHlen)?;
                }

                reply
            },
            hops: bytes.byte()?,
            xid: u32::from_be_bytes(bytes.arr()?),
            secs: u16::from_be_bytes(bytes.arr()?),
            broadcast: u16::from_be_bytes(bytes.arr()?) & 128 != 0,
            ciaddr: bytes.arr()?.into(),
            yiaddr: bytes.arr()?.into(),
            siaddr: bytes.arr()?.into(),
            giaddr: bytes.arr()?.into(),
            chaddr: bytes.arr()?,
            options: {
                for _ in 0..Self::SERVER_NAME_AND_FILE_NAME {
                    bytes.byte()?;
                }

                if bytes.arr()? != Self::COOKIE {
                    Err(Error::MissingCookie)?;
                }

                Options(OptionsInner::decode(bytes.remaining())?)
            },
        })
    }

    /// Encodes the packet into the provided buf slice
    pub fn encode<'o>(&self, buf: &'o mut [u8]) -> Result<&'o [u8], Error> {
        let mut bytes = BytesOut::new(buf);

        bytes
            .push(&[if self.reply {
                Self::BOOT_REPLY
            } else {
                Self::BOOT_REQUEST
            }])?
            .byte(1)?
            .byte(6)?
            .byte(self.hops)?
            .push(&u32::to_be_bytes(self.xid))?
            .push(&u16::to_be_bytes(self.secs))?
            .push(&u16::to_be_bytes(if self.broadcast { 128 } else { 0 }))?
            .push(&self.ciaddr.octets())?
            .push(&self.yiaddr.octets())?
            .push(&self.siaddr.octets())?
            .push(&self.giaddr.octets())?
            .push(&self.chaddr)?;

        for _ in 0..Self::SERVER_NAME_AND_FILE_NAME {
            bytes.byte(0)?;
        }

        bytes.push(&Self::COOKIE)?;

        self.options.0.encode(&mut bytes)?;

        bytes.byte(Self::END)?;

        while bytes.len() < 272 {
            bytes.byte(Self::PAD)?;
        }

        let len = bytes.len();

        Ok(&buf[..len])
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Settings<'a> {
    pub ip: Ipv4Addr,
    pub server_ip: Option<Ipv4Addr>,
    pub lease_time_secs: Option<u32>,
    pub gateway: Option<Ipv4Addr>,
    pub subnet: Option<Ipv4Addr>,
    pub dns1: Option<Ipv4Addr>,
    pub dns2: Option<Ipv4Addr>,
    pub captive_url: Option<&'a str>,
}

impl<'a> Settings<'a> {
    pub fn new(packet: &Packet<'a>) -> Self {
        Self {
            ip: packet.yiaddr,
            server_ip: packet.options.iter().find_map(|option| {
                if let DhcpOption::ServerIdentifier(ip) = option {
                    Some(ip)
                } else {
                    None
                }
            }),
            lease_time_secs: packet.options.iter().find_map(|option| {
                if let DhcpOption::IpAddressLeaseTime(lease_time_secs) = option {
                    Some(lease_time_secs)
                } else {
                    None
                }
            }),
            gateway: packet.options.iter().find_map(|option| {
                if let DhcpOption::Router(ips) = option {
                    ips.iter().next()
                } else {
                    None
                }
            }),
            subnet: packet.options.iter().find_map(|option| {
                if let DhcpOption::SubnetMask(subnet) = option {
                    Some(subnet)
                } else {
                    None
                }
            }),
            dns1: packet.options.iter().find_map(|option| {
                if let DhcpOption::DomainNameServer(ips) = option {
                    ips.iter().next()
                } else {
                    None
                }
            }),
            dns2: packet.options.iter().find_map(|option| {
                if let DhcpOption::DomainNameServer(ips) = option {
                    ips.iter().nth(1)
                } else {
                    None
                }
            }),
            captive_url: packet.options.iter().find_map(|option| {
                if let DhcpOption::CaptiveUrl(url) = option {
                    Some(url)
                } else {
                    None
                }
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Options<'a>(OptionsInner<'a>);

impl<'a> Options<'a> {
    const REQUEST_PARAMS: &'static [u8] = &[
        DhcpOption::CODE_ROUTER,
        DhcpOption::CODE_SUBNET,
        DhcpOption::CODE_DNS,
    ];

    pub const fn new(options: &'a [DhcpOption<'a>]) -> Self {
        Self(OptionsInner::DataSlice(options))
    }

    #[inline(always)]
    pub const fn buf() -> [DhcpOption<'a>; 8] {
        [DhcpOption::Message(""); 8]
    }

    pub fn discover(requested_ip: Option<Ipv4Addr>, buf: &'a mut [DhcpOption<'a>]) -> Self {
        buf[0] = DhcpOption::MessageType(MessageType::Discover);

        let mut offset = 1;

        if let Some(requested_ip) = requested_ip {
            buf[1] = DhcpOption::RequestedIpAddress(requested_ip);
            offset += 1;
        }

        Self::new(&buf[..offset])
    }

    pub fn request(ip: Ipv4Addr, buf: &'a mut [DhcpOption<'a>]) -> Self {
        buf[0] = DhcpOption::MessageType(MessageType::Request);
        buf[1] = DhcpOption::RequestedIpAddress(ip);
        buf[2] = DhcpOption::ParameterRequestList(Self::REQUEST_PARAMS);

        Self::new(&buf[..3])
    }

    pub fn release(buf: &'a mut [DhcpOption<'a>]) -> Self {
        buf[0] = DhcpOption::MessageType(MessageType::Release);

        Self::new(&buf[..1])
    }

    pub fn decline(buf: &'a mut [DhcpOption<'a>]) -> Self {
        buf[0] = DhcpOption::MessageType(MessageType::Decline);

        Self::new(&buf[..1])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reply<'b>(
        &self,
        mt: MessageType,
        server_ip: Ipv4Addr,
        lease_duration_secs: u32,
        gateways: &'b [Ipv4Addr],
        subnet: Option<Ipv4Addr>,
        dns: &'b [Ipv4Addr],
        captive_url: Option<&'b str>,
        buf: &'b mut [DhcpOption<'b>],
    ) -> Options<'b> {
        let requested = self.iter().find_map(|option| {
            if let DhcpOption::ParameterRequestList(requested) = option {
                Some(requested)
            } else {
                None
            }
        });

        Options::internal_reply(
            requested,
            mt,
            server_ip,
            lease_duration_secs,
            gateways,
            subnet,
            dns,
            captive_url,
            buf,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn internal_reply(
        requested: Option<&[u8]>,
        mt: MessageType,
        server_ip: Ipv4Addr,
        lease_duration_secs: u32,
        gateways: &'a [Ipv4Addr],
        subnet: Option<Ipv4Addr>,
        dns: &'a [Ipv4Addr],
        captive_url: Option<&'a str>,
        buf: &'a mut [DhcpOption<'a>],
    ) -> Self {
        buf[0] = DhcpOption::MessageType(mt);
        buf[1] = DhcpOption::ServerIdentifier(server_ip);
        buf[2] = DhcpOption::IpAddressLeaseTime(lease_duration_secs);

        let mut offset = 3;

        if !matches!(mt, MessageType::Nak) {
            if let Some(requested) = requested {
                for code in requested {
                    if !buf[0..offset].iter().any(|option| option.code() == *code) {
                        let option = match *code {
                            DhcpOption::CODE_ROUTER => (!gateways.is_empty())
                                .then_some(DhcpOption::Router(Ipv4Addrs::new(gateways))),
                            DhcpOption::CODE_DNS => (!dns.is_empty())
                                .then_some(DhcpOption::DomainNameServer(Ipv4Addrs::new(dns))),
                            DhcpOption::CODE_SUBNET => subnet.map(DhcpOption::SubnetMask),
                            DhcpOption::CODE_CAPTIVE_URL => captive_url.map(DhcpOption::CaptiveUrl),
                            _ => None,
                        };

                        if let Some(option) = option {
                            buf[offset] = option;
                            offset += 1;
                        }
                    }

                    if offset == buf.len() {
                        break;
                    }
                }
            }
        }

        Self::new(&buf[..offset])
    }

    pub fn iter(&self) -> impl Iterator<Item = DhcpOption<'a>> + 'a {
        self.0.iter()
    }

    pub(crate) fn requested_ip(&self) -> Option<Ipv4Addr> {
        self.iter().find_map(|option| {
            if let DhcpOption::RequestedIpAddress(ip) = option {
                Some(ip)
            } else {
                None
            }
        })
    }
}

impl core::fmt::Debug for Options<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum OptionsInner<'a> {
    ByteSlice(&'a [u8]),
    DataSlice(&'a [DhcpOption<'a>]),
}

impl<'a> OptionsInner<'a> {
    fn decode(data: &'a [u8]) -> Result<Self, Error> {
        let mut bytes = BytesIn::new(data);

        while DhcpOption::decode(&mut bytes)?.is_some() {}

        Ok(Self::ByteSlice(data))
    }

    fn encode(&self, buf: &mut BytesOut) -> Result<(), Error> {
        for option in self.iter() {
            option.encode(buf)?;
        }

        Ok(())
    }

    fn iter(&self) -> impl Iterator<Item = DhcpOption<'a>> + 'a {
        struct ByteSliceDhcpOptions<'a>(BytesIn<'a>);

        impl<'a> Iterator for ByteSliceDhcpOptions<'a> {
            type Item = DhcpOption<'a>;

            fn next(&mut self) -> Option<Self::Item> {
                if self.0.is_empty() {
                    None
                } else {
                    unwrap!(DhcpOption::decode(&mut self.0))
                }
            }
        }

        match self {
            Self::ByteSlice(data) => {
                EitherIterator::First(ByteSliceDhcpOptions(BytesIn::new(data)))
            }
            Self::DataSlice(data) => EitherIterator::Second(data.iter().cloned()),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DhcpOption<'a> {
    /// 53: DHCP Message Type
    MessageType(MessageType),
    /// 54: Server Identifier
    ServerIdentifier(Ipv4Addr),
    /// 55: Parameter Request List
    ParameterRequestList(&'a [u8]),
    /// 50: Requested IP Address
    RequestedIpAddress(Ipv4Addr),
    /// 12: Host Name Option
    HostName(&'a str),
    /// 3: Router Option
    Router(Ipv4Addrs<'a>),
    /// 6: Domain Name Server Option
    DomainNameServer(Ipv4Addrs<'a>),
    /// 51: IP Address Lease Time
    IpAddressLeaseTime(u32),
    /// 1: Subnet Mask
    SubnetMask(Ipv4Addr),
    /// 56: Message
    Message(&'a str),
    /// 57: Maximum DHCP Message Size
    MaximumMessageSize(u16),
    /// 61: Client-identifier
    ClientIdentifier(&'a [u8]),
    /// 114: Captive-portal URL
    CaptiveUrl(&'a str),
    // Other (unrecognized)
    Unrecognized(u8, &'a [u8]),
}

impl DhcpOption<'_> {
    pub const CODE_ROUTER: u8 = DhcpOption::Router(Ipv4Addrs::new(&[])).code();
    pub const CODE_DNS: u8 = DhcpOption::DomainNameServer(Ipv4Addrs::new(&[])).code();
    pub const CODE_SUBNET: u8 = DhcpOption::SubnetMask(Ipv4Addr::new(0, 0, 0, 0)).code();
    pub const CODE_CAPTIVE_URL: u8 = DhcpOption::CaptiveUrl("").code();

    fn decode<'o>(bytes: &mut BytesIn<'o>) -> Result<Option<DhcpOption<'o>>, Error> {
        let code = bytes.byte()?;
        if code == Packet::END {
            Ok(None)
        } else {
            let len = bytes.byte()? as usize;
            let mut bytes = BytesIn::new(bytes.slice(len)?);

            let option = match code {
                DHCP_MESSAGE_TYPE => DhcpOption::MessageType(
                    TryFromPrimitive::try_from_primitive(bytes.remaining_byte()?)
                        .map_err(|_| Error::InvalidMessageType)?,
                ),
                SERVER_IDENTIFIER => {
                    DhcpOption::ServerIdentifier(Ipv4Addr::from(bytes.remaining_arr()?))
                }
                PARAMETER_REQUEST_LIST => DhcpOption::ParameterRequestList(bytes.remaining()),
                REQUESTED_IP_ADDRESS => {
                    DhcpOption::RequestedIpAddress(Ipv4Addr::from(bytes.remaining_arr()?))
                }
                HOST_NAME => DhcpOption::HostName(
                    core::str::from_utf8(bytes.remaining()).map_err(Error::InvalidUtf8Str)?,
                ),
                MAXIMUM_DHCP_MESSAGE_SIZE => {
                    DhcpOption::MaximumMessageSize(u16::from_be_bytes(bytes.remaining_arr()?))
                }
                ROUTER => {
                    DhcpOption::Router(Ipv4Addrs(Ipv4AddrsInner::ByteSlice(bytes.remaining())))
                }
                DOMAIN_NAME_SERVER => DhcpOption::DomainNameServer(Ipv4Addrs(
                    Ipv4AddrsInner::ByteSlice(bytes.remaining()),
                )),
                IP_ADDRESS_LEASE_TIME => {
                    DhcpOption::IpAddressLeaseTime(u32::from_be_bytes(bytes.remaining_arr()?))
                }
                SUBNET_MASK => DhcpOption::SubnetMask(Ipv4Addr::from(bytes.remaining_arr()?)),
                MESSAGE => DhcpOption::Message(
                    core::str::from_utf8(bytes.remaining()).map_err(Error::InvalidUtf8Str)?,
                ),
                CLIENT_IDENTIFIER => {
                    if len < 2 {
                        return Err(Error::DataUnderflow);
                    }

                    DhcpOption::ClientIdentifier(bytes.remaining())
                }
                CAPTIVE_URL => DhcpOption::HostName(
                    core::str::from_utf8(bytes.remaining()).map_err(Error::InvalidUtf8Str)?,
                ),
                _ => DhcpOption::Unrecognized(code, bytes.remaining()),
            };

            Ok(Some(option))
        }
    }

    fn encode(&self, out: &mut BytesOut) -> Result<(), Error> {
        out.byte(self.code())?;

        self.data(|data| {
            out.byte(data.len() as _)?;
            out.push(data)?;

            Ok(())
        })
    }

    pub const fn code(&self) -> u8 {
        match self {
            Self::MessageType(_) => DHCP_MESSAGE_TYPE,
            Self::ServerIdentifier(_) => SERVER_IDENTIFIER,
            Self::ParameterRequestList(_) => PARAMETER_REQUEST_LIST,
            Self::RequestedIpAddress(_) => REQUESTED_IP_ADDRESS,
            Self::HostName(_) => HOST_NAME,
            Self::Router(_) => ROUTER,
            Self::DomainNameServer(_) => DOMAIN_NAME_SERVER,
            Self::IpAddressLeaseTime(_) => IP_ADDRESS_LEASE_TIME,
            Self::SubnetMask(_) => SUBNET_MASK,
            Self::MaximumMessageSize(_) => MAXIMUM_DHCP_MESSAGE_SIZE,
            Self::Message(_) => MESSAGE,
            Self::ClientIdentifier(_) => CLIENT_IDENTIFIER,
            Self::CaptiveUrl(_) => CAPTIVE_URL,
            Self::Unrecognized(code, _) => *code,
        }
    }

    fn data(&self, mut f: impl FnMut(&[u8]) -> Result<(), Error>) -> Result<(), Error> {
        match self {
            Self::MessageType(mtype) => f(&[*mtype as _]),
            Self::ServerIdentifier(addr) => f(&addr.octets()),
            Self::ParameterRequestList(prl) => f(prl),
            Self::RequestedIpAddress(addr) => f(&addr.octets()),
            Self::HostName(name) => f(name.as_bytes()),
            Self::Router(addrs) | Self::DomainNameServer(addrs) => {
                for addr in addrs.iter() {
                    f(&addr.octets())?;
                }

                Ok(())
            }
            Self::IpAddressLeaseTime(secs) => f(&secs.to_be_bytes()),
            Self::SubnetMask(mask) => f(&mask.octets()),
            Self::Message(msg) => f(msg.as_bytes()),
            Self::MaximumMessageSize(size) => f(&size.to_be_bytes()),
            Self::ClientIdentifier(id) => f(id),
            Self::CaptiveUrl(name) => f(name.as_bytes()),
            Self::Unrecognized(_, data) => f(data),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Ipv4Addrs<'a>(Ipv4AddrsInner<'a>);

impl<'a> Ipv4Addrs<'a> {
    pub const fn new(addrs: &'a [Ipv4Addr]) -> Self {
        Self(Ipv4AddrsInner::DataSlice(addrs))
    }

    pub fn iter(&self) -> impl Iterator<Item = Ipv4Addr> + 'a {
        self.0.iter()
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum Ipv4AddrsInner<'a> {
    ByteSlice(&'a [u8]),
    DataSlice(&'a [Ipv4Addr]),
}

impl<'a> Ipv4AddrsInner<'a> {
    fn iter(&self) -> impl Iterator<Item = Ipv4Addr> + 'a {
        match self {
            Self::ByteSlice(data) => {
                EitherIterator::First((0..data.len()).step_by(4).map(|offset| {
                    let octets: [u8; 4] = unwrap!(data[offset..offset + 4].try_into());

                    octets.into()
                }))
            }
            Self::DataSlice(data) => EitherIterator::Second(data.iter().cloned()),
        }
    }
}

enum EitherIterator<F, S> {
    First(F),
    Second(S),
}

impl<F, S> Iterator for EitherIterator<F, S>
where
    F: Iterator,
    S: Iterator<Item = F::Item>,
{
    type Item = F::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::First(iter) => iter.next(),
            Self::Second(iter) => iter.next(),
        }
    }
}

// DHCP Options
const SUBNET_MASK: u8 = 1;
const ROUTER: u8 = 3;
const DOMAIN_NAME_SERVER: u8 = 6;
const HOST_NAME: u8 = 12;

// DHCP Extensions
const REQUESTED_IP_ADDRESS: u8 = 50;
const IP_ADDRESS_LEASE_TIME: u8 = 51;
const DHCP_MESSAGE_TYPE: u8 = 53;
const SERVER_IDENTIFIER: u8 = 54;
const PARAMETER_REQUEST_LIST: u8 = 55;
const MESSAGE: u8 = 56;
const MAXIMUM_DHCP_MESSAGE_SIZE: u8 = 57;
const CLIENT_IDENTIFIER: u8 = 61;
const CAPTIVE_URL: u8 = 114;
