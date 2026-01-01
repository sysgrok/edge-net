use core::net::Ipv4Addr;

use super::bytes::{BytesIn, BytesOut};

use super::{checksum_accumulate, checksum_finish, Error};

#[allow(clippy::type_complexity)]
pub fn decode(
    packet: &[u8],
    filter_src: Ipv4Addr,
    filter_dst: Ipv4Addr,
    filter_proto: Option<u8>,
) -> Result<Option<(Ipv4Addr, Ipv4Addr, u8, &[u8])>, Error> {
    let data = Ipv4PacketHeader::decode_with_payload(packet, filter_src, filter_dst, filter_proto)?
        .map(|(hdr, payload)| (hdr.src, hdr.dst, hdr.p, payload));

    Ok(data)
}

pub fn encode<F>(
    buf: &mut [u8],
    src: Ipv4Addr,
    dst: Ipv4Addr,
    proto: u8,
    encoder: F,
) -> Result<&[u8], Error>
where
    F: FnOnce(&mut [u8]) -> Result<usize, Error>,
{
    let mut hdr = Ipv4PacketHeader::new(src, dst, proto);

    hdr.encode_with_payload(buf, encoder)
}

/// Represents a parsed IP header
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Ipv4PacketHeader {
    /// Version
    pub version: u8,
    /// Header length
    pub hlen: u8,
    /// Type of service
    pub tos: u8,
    /// Total length
    pub len: u16,
    /// Identification
    pub id: u16,
    /// Fragment offset field
    pub off: u16,
    /// Time to live
    pub ttl: u8,
    /// Protocol
    pub p: u8,
    /// Checksum
    pub sum: u16,
    /// Source address
    pub src: Ipv4Addr,
    /// Dest address
    pub dst: Ipv4Addr,
}

impl Ipv4PacketHeader {
    pub const MIN_SIZE: usize = 20;
    pub const CHECKSUM_WORD: usize = 5;

    pub const IP_DF: u16 = 0x4000; // Don't fragment flag
    pub const IP_MF: u16 = 0x2000; // More fragments flag

    /// Create a new header instance
    pub fn new(src: Ipv4Addr, dst: Ipv4Addr, proto: u8) -> Self {
        Self {
            version: 4,
            hlen: Self::MIN_SIZE as _,
            tos: 0,
            len: Self::MIN_SIZE as _,
            id: 0,
            off: 0,
            ttl: 64,
            p: proto,
            sum: 0,
            src,
            dst,
        }
    }

    /// Decodes the header from a byte slice
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut bytes = BytesIn::new(data);

        let vhl = bytes.byte()?;

        Ok(Self {
            version: vhl >> 4,
            hlen: (vhl & 0x0f) * 4,
            tos: bytes.byte()?,
            len: u16::from_be_bytes(bytes.arr()?),
            id: u16::from_be_bytes(bytes.arr()?),
            off: u16::from_be_bytes(bytes.arr()?),
            ttl: bytes.byte()?,
            p: bytes.byte()?,
            sum: u16::from_be_bytes(bytes.arr()?),
            src: u32::from_be_bytes(bytes.arr()?).into(),
            dst: u32::from_be_bytes(bytes.arr()?).into(),
        })
    }

    /// Encodes the header into the provided buf slice
    pub fn encode<'o>(&self, buf: &'o mut [u8]) -> Result<&'o [u8], Error> {
        let mut bytes = BytesOut::new(buf);

        bytes
            .byte(
                (self.version << 4)
                    | (self.hlen / 4 + (if !self.hlen.is_multiple_of(4) { 1 } else { 0 })),
            )?
            .byte(self.tos)?
            .push(&u16::to_be_bytes(self.len))?
            .push(&u16::to_be_bytes(self.id))?
            .push(&u16::to_be_bytes(self.off))?
            .byte(self.ttl)?
            .byte(self.p)?
            .push(&u16::to_be_bytes(self.sum))?
            .push(&u32::to_be_bytes(self.src.into()))?
            .push(&u32::to_be_bytes(self.dst.into()))?;

        let len = bytes.len();

        Ok(&buf[..len])
    }

    /// Encodes the header and the provided payload into the provided buf slice
    pub fn encode_with_payload<'o, F>(
        &mut self,
        buf: &'o mut [u8],
        encoder: F,
    ) -> Result<&'o [u8], Error>
    where
        F: FnOnce(&mut [u8]) -> Result<usize, Error>,
    {
        let hdr_len = self.hlen as usize;
        if hdr_len < Self::MIN_SIZE || buf.len() < hdr_len {
            Err(Error::BufferOverflow)?;
        }

        let (hdr_buf, payload_buf) = buf.split_at_mut(hdr_len);

        let payload_len = encoder(payload_buf)?;

        let len = hdr_len + payload_len;
        self.len = len as _;

        let min_hdr_len = self.encode(hdr_buf)?.len();
        assert_eq!(min_hdr_len, Self::MIN_SIZE);

        hdr_buf[Self::MIN_SIZE..hdr_len].fill(0);

        let checksum = Self::checksum(hdr_buf);
        self.sum = checksum;

        Self::inject_checksum(hdr_buf, checksum);

        Ok(&buf[..len])
    }

    /// Decodes the provided packet into a header and a payload slice
    pub fn decode_with_payload(
        packet: &[u8],
        filter_src: Ipv4Addr,
        filter_dst: Ipv4Addr,
        filter_proto: Option<u8>,
    ) -> Result<Option<(Self, &[u8])>, Error> {
        let hdr = Self::decode(packet)?;
        if hdr.version == 4 {
            // IPv4

            if !filter_src.is_unspecified() && !hdr.src.is_broadcast() && filter_src != hdr.src {
                return Ok(None);
            }

            if !filter_dst.is_unspecified() && !hdr.dst.is_broadcast() && filter_dst != hdr.dst {
                return Ok(None);
            }

            if let Some(filter_proto) = filter_proto {
                if filter_proto != hdr.p {
                    return Ok(None);
                }
            }

            let len = hdr.len as usize;
            if packet.len() < len {
                Err(Error::DataUnderflow)?;
            }

            let checksum = Self::checksum(&packet[..len]);

            trace!("IP header decoded, total_size={}, src={}, dst={}, hlen={}, size={}, checksum={}, ours={}", packet.len(), hdr.src, hdr.dst, hdr.hlen, hdr.len, hdr.sum, checksum);

            if checksum != hdr.sum {
                Err(Error::InvalidChecksum)?;
            }

            let packet = &packet[..len];
            let hdr_len = hdr.hlen as usize;
            if packet.len() < hdr_len {
                Err(Error::DataUnderflow)?;
            }

            Ok(Some((hdr, &packet[hdr_len..])))
        } else {
            Err(Error::InvalidFormat)
        }
    }

    /// Injects the checksum into the provided packet
    pub fn inject_checksum(packet: &mut [u8], checksum: u16) {
        let checksum = checksum.to_be_bytes();

        let offset = Self::CHECKSUM_WORD << 1;
        packet[offset] = checksum[0];
        packet[offset + 1] = checksum[1];
    }

    /// Computes the checksum for an already encoded packet
    pub fn checksum(packet: &[u8]) -> u16 {
        let hlen = (packet[0] & 0x0f) as usize * 4;

        let sum = checksum_accumulate(&packet[..hlen], Self::CHECKSUM_WORD);

        checksum_finish(sum)
    }
}
