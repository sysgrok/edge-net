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
