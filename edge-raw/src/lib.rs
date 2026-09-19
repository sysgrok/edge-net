#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::net::{Ipv4Addr, SocketAddrV4};

use self::udp::UdpPacketHeader;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "io")]
pub mod io;

pub mod bytes;
pub mod ip;
pub mod udp;

use bytes::BytesIn;

/// An error type for decoding and encoding IP and UDP oackets
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Error {
    DataUnderflow,
    BufferOverflow,
    InvalidFormat,
    InvalidChecksum,
}

impl From<bytes::Error> for Error {
    fn from(value: bytes::Error) -> Self {
        match value {
            bytes::Error::BufferOverflow => Self::BufferOverflow,
            bytes::Error::DataUnderflow => Self::DataUnderflow,
            bytes::Error::InvalidFormat => Self::InvalidFormat,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let str = match self {
            Self::DataUnderflow => "Data underflow",
            Self::BufferOverflow => "Buffer overflow",
            Self::InvalidFormat => "Invalid format",
            Self::InvalidChecksum => "Invalid checksum",
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
            Self::InvalidFormat => "Invalid format",
            Self::InvalidChecksum => "Invalid checksum",
        };

        defmt::write!(f, "{}", str)
    }
}

impl core::error::Error for Error {}

/// Decodes an IP packet and its UDP payload
#[allow(clippy::type_complexity)]
pub fn ip_udp_decode(
    packet: &[u8],
    filter_src: Option<SocketAddrV4>,
    filter_dst: Option<SocketAddrV4>,
) -> Result<Option<(SocketAddrV4, SocketAddrV4, &[u8])>, Error> {
    if let Some((src, dst, _proto, udp_packet)) = ip::decode(
        packet,
        filter_src.map(|a| *a.ip()).unwrap_or(Ipv4Addr::UNSPECIFIED),
        filter_dst.map(|a| *a.ip()).unwrap_or(Ipv4Addr::UNSPECIFIED),
        Some(UdpPacketHeader::PROTO),
    )? {
        udp::decode(
            src,
            dst,
            udp_packet,
            filter_src.map(|a| a.port()),
            filter_dst.map(|a| a.port()),
        )
    } else {
        Ok(None)
    }
}

/// Encodes an IP packet and its UDP payload
pub fn ip_udp_encode<F>(
    buf: &mut [u8],
    src: SocketAddrV4,
    dst: SocketAddrV4,
    encoder: F,
) -> Result<&[u8], Error>
where
    F: FnOnce(&mut [u8]) -> Result<usize, Error>,
{
    ip::encode(buf, *src.ip(), *dst.ip(), UdpPacketHeader::PROTO, |buf| {
        Ok(udp::encode(buf, src, dst, encoder)?.len())
    })
}

pub fn checksum_accumulate(bytes: &[u8], checksum_word: usize) -> u32 {
    let mut bytes = BytesIn::new(bytes);

    let mut sum: u32 = 0;
    while !bytes.is_empty() {
        let skip = (bytes.offset() >> 1) == checksum_word;
        let arr = bytes
            .arr()
            .ok()
            .unwrap_or_else(|| [unwrap!(bytes.byte(), "Unreachable"), 0]);

        let word = if skip { 0 } else { u16::from_be_bytes(arr) };

        sum += word as u32;
    }

    sum
}

pub fn checksum_finish(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum >> 16) + (sum & 0xffff);
    }

    !sum as u16
}

#[cfg(test)]
mod test {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use super::ip::Ipv4PacketHeader;
    use super::udp::UdpPacketHeader;
    use super::{checksum_accumulate, checksum_finish, ip, ip_udp_decode, ip_udp_encode, Error};

    const SRC: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 68);
    const DST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 2), 67);
    const PAYLOAD: &[u8] = b"hello";

    fn encode(buf: &mut [u8]) -> usize {
        ip_udp_encode(buf, SRC, DST, |payload| {
            payload[..PAYLOAD.len()].copy_from_slice(PAYLOAD);
            Ok(PAYLOAD.len())
        })
        .unwrap()
        .len()
    }

    #[test]
    fn checksum() {
        // The example from RFC 1071
        let bytes = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(checksum_accumulate(&bytes, usize::MAX), 0x2ddf0);
        assert_eq!(checksum_finish(0x2ddf0), 0x220d);

        // An odd trailing byte is padded with zero
        assert_eq!(
            checksum_accumulate(&[0x12, 0x34, 0x56], usize::MAX),
            0x1234 + 0x5600
        );

        // The checksum word itself is skipped
        assert_eq!(checksum_accumulate(&[0x12, 0x34, 0x56, 0x78], 1), 0x1234);

        // Carries are folded until the sum fits in 16 bits
        assert_eq!(checksum_finish(0x1_ffff), 0xfffe);
    }

    #[test]
    fn roundtrip() {
        let mut buf = [0u8; 64];
        let len = encode(&mut buf);
        assert_eq!(
            len,
            Ipv4PacketHeader::MIN_SIZE + UdpPacketHeader::SIZE + PAYLOAD.len()
        );

        let (src, dst, payload) = ip_udp_decode(&buf[..len], None, None).unwrap().unwrap();
        assert_eq!(src, SRC);
        assert_eq!(dst, DST);
        assert_eq!(payload, PAYLOAD);

        // Both headers carry valid lengths and checksums
        let hdr = Ipv4PacketHeader::decode(&buf[..len]).unwrap();
        assert_eq!(hdr.version, 4);
        assert_eq!(hdr.hlen as usize, Ipv4PacketHeader::MIN_SIZE);
        assert_eq!(hdr.len as usize, len);
        assert_eq!(hdr.p, UdpPacketHeader::PROTO);
        assert_eq!(hdr.src, *SRC.ip());
        assert_eq!(hdr.dst, *DST.ip());
        assert_eq!(hdr.sum, Ipv4PacketHeader::checksum(&buf[..len]));

        let udp = &buf[Ipv4PacketHeader::MIN_SIZE..len];
        let hdr = UdpPacketHeader::decode(udp).unwrap();
        assert_eq!(hdr.src, SRC.port());
        assert_eq!(hdr.dst, DST.port());
        assert_eq!(hdr.len as usize, UdpPacketHeader::SIZE + PAYLOAD.len());
        assert_eq!(
            hdr.sum,
            UdpPacketHeader::checksum(udp, *SRC.ip(), *DST.ip())
        );
    }

    #[test]
    fn filters() {
        let mut buf = [0u8; 64];
        let len = encode(&mut buf);
        let packet = &buf[..len];

        assert!(ip_udp_decode(packet, Some(SRC), Some(DST))
            .unwrap()
            .is_some());
        assert!(ip_udp_decode(packet, Some(SRC), None).unwrap().is_some());

        // Mismatching address or port filters skip the packet without an error
        let other_ip = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 3), SRC.port());
        let other_port = SocketAddrV4::new(*DST.ip(), 1);
        assert!(ip_udp_decode(packet, Some(other_ip), None)
            .unwrap()
            .is_none());
        assert!(ip_udp_decode(packet, None, Some(other_port))
            .unwrap()
            .is_none());

        // So does a mismatching protocol filter
        let any = Ipv4Addr::UNSPECIFIED;
        assert!(ip::decode(packet, any, any, Some(6)).unwrap().is_none());
        assert!(ip::decode(packet, any, any, Some(UdpPacketHeader::PROTO))
            .unwrap()
            .is_some());
    }

    #[test]
    fn errors() {
        let mut buf = [0u8; 64];
        let len = encode(&mut buf);

        // Truncated packet
        assert_eq!(
            ip_udp_decode(&buf[..len - 1], None, None).err(),
            Some(Error::DataUnderflow)
        );

        // Not IPv4
        let mut packet = buf;
        packet[0] = 0x65;
        assert_eq!(
            ip_udp_decode(&packet[..len], None, None).err(),
            Some(Error::InvalidFormat)
        );

        // Corrupted IP header
        let mut packet = buf;
        packet[8] ^= 1;
        assert_eq!(
            ip_udp_decode(&packet[..len], None, None).err(),
            Some(Error::InvalidChecksum)
        );

        // Corrupted UDP payload
        let mut packet = buf;
        packet[len - 1] ^= 1;
        assert_eq!(
            ip_udp_decode(&packet[..len], None, None).err(),
            Some(Error::InvalidChecksum)
        );

        // No room for the headers
        assert_eq!(
            ip_udp_encode(&mut [0u8; 10], SRC, DST, |_| Ok(0)).err(),
            Some(Error::BufferOverflow)
        );

        // A payload encoder error is passed through
        assert_eq!(
            ip_udp_encode(&mut buf, SRC, DST, |_| Err(Error::InvalidFormat)).err(),
            Some(Error::InvalidFormat)
        );
    }
}
