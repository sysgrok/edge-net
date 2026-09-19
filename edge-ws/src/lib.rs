#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

pub type Fragmented = bool;
pub type Final = bool;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "io")]
pub mod io;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FrameType {
    Text(Fragmented),
    Binary(Fragmented),
    Ping,
    Pong,
    Close,
    Continue(Final),
}

impl FrameType {
    pub fn is_fragmented(&self) -> bool {
        match self {
            Self::Text(fragmented) | Self::Binary(fragmented) => *fragmented,
            Self::Continue(_) => true,
            _ => false,
        }
    }

    pub fn is_final(&self) -> bool {
        match self {
            Self::Text(fragmented) | Self::Binary(fragmented) => !*fragmented,
            Self::Continue(final_) => *final_,
            _ => true,
        }
    }
}

impl core::fmt::Display for FrameType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Text(fragmented) => {
                write!(f, "Text{}", if *fragmented { " (fragmented)" } else { "" })
            }
            Self::Binary(fragmented) => write!(
                f,
                "Binary{}",
                if *fragmented { " (fragmented)" } else { "" }
            ),
            Self::Ping => write!(f, "Ping"),
            Self::Pong => write!(f, "Pong"),
            Self::Close => write!(f, "Close"),
            Self::Continue(ffinal) => {
                write!(f, "Continue{}", if *ffinal { " (final)" } else { "" })
            }
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for FrameType {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Text(fragmented) => {
                defmt::write!(f, "Text{}", if *fragmented { " (fragmented)" } else { "" })
            }
            Self::Binary(fragmented) => defmt::write!(
                f,
                "Binary{}",
                if *fragmented { " (fragmented)" } else { "" }
            ),
            Self::Ping => defmt::write!(f, "Ping"),
            Self::Pong => defmt::write!(f, "Pong"),
            Self::Close => defmt::write!(f, "Close"),
            Self::Continue(ffinal) => {
                defmt::write!(f, "Continue{}", if *ffinal { " (final)" } else { "" })
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Error<E> {
    Incomplete(usize),
    Invalid,
    BufferOverflow,
    InvalidLen,
    Io(E),
}

impl Error<()> {
    pub fn recast<E>(self) -> Error<E> {
        match self {
            Self::Incomplete(v) => Error::Incomplete(v),
            Self::Invalid => Error::Invalid,
            Self::BufferOverflow => Error::BufferOverflow,
            Self::InvalidLen => Error::InvalidLen,
            Self::Io(_) => panic!(),
        }
    }
}

impl<E> core::fmt::Display for Error<E>
where
    E: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Incomplete(size) => write!(f, "Incomplete: {} bytes missing", size),
            Self::Invalid => write!(f, "Invalid"),
            Self::BufferOverflow => write!(f, "Buffer overflow"),
            Self::InvalidLen => write!(f, "Invalid length"),
            Self::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

#[cfg(feature = "defmt")]
impl<E> defmt::Format for Error<E>
where
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Incomplete(size) => defmt::write!(f, "Incomplete: {} bytes missing", size),
            Self::Invalid => defmt::write!(f, "Invalid"),
            Self::BufferOverflow => defmt::write!(f, "Buffer overflow"),
            Self::InvalidLen => defmt::write!(f, "Invalid length"),
            Self::Io(err) => defmt::write!(f, "IO error: {}", err),
        }
    }
}

impl<E> core::error::Error for Error<E> where E: core::error::Error {}

#[derive(Clone, Debug)]
pub struct FrameHeader {
    pub frame_type: FrameType,
    pub payload_len: u64,
    pub mask_key: Option<u32>,
}

impl FrameHeader {
    pub const MIN_LEN: usize = 2;
    pub const MAX_LEN: usize = FrameHeader {
        frame_type: FrameType::Binary(false),
        payload_len: 65536,
        mask_key: Some(0),
    }
    .serialized_len();

    pub fn deserialize(buf: &[u8]) -> Result<(Self, usize), Error<()>> {
        let mut expected_len = 2_usize;

        if buf.len() < expected_len {
            Err(Error::Incomplete(expected_len - buf.len()))
        } else {
            let final_frame = buf[0] & 0x80 != 0;

            let rsv = buf[0] & 0x70;
            if rsv != 0 {
                return Err(Error::Invalid);
            }

            let opcode = buf[0] & 0x0f;
            if (3..=7).contains(&opcode) || opcode >= 11 {
                return Err(Error::Invalid);
            }

            let mut payload_len = (buf[1] & 0x7f) as u64;
            let mut payload_offset = 2;

            if payload_len == 126 {
                expected_len += 2;

                if buf.len() < expected_len {
                    return Err(Error::Incomplete(expected_len - buf.len()));
                } else {
                    payload_len = u16::from_be_bytes([buf[2], buf[3]]) as _;
                    payload_offset += 2;
                }
            } else if payload_len == 127 {
                expected_len += 8;

                if buf.len() < expected_len {
                    return Err(Error::Incomplete(expected_len - buf.len()));
                } else {
                    payload_len = u64::from_be_bytes([
                        buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
                    ]);
                    payload_offset += 8;
                }
            }

            let masked = buf[1] & 0x80 != 0;
            let mask_key = if masked {
                expected_len += 4;
                if buf.len() < expected_len {
                    return Err(Error::Incomplete(expected_len - buf.len()));
                } else {
                    let mask_key = Some(u32::from_be_bytes([
                        buf[payload_offset],
                        buf[payload_offset + 1],
                        buf[payload_offset + 2],
                        buf[payload_offset + 3],
                    ]));
                    payload_offset += 4;

                    mask_key
                }
            } else {
                None
            };

            let frame_type = match opcode {
                0 => FrameType::Continue(final_frame),
                1 => FrameType::Text(!final_frame),
                2 => FrameType::Binary(!final_frame),
                8 => FrameType::Close,
                9 => FrameType::Ping,
                10 => FrameType::Pong,
                _ => unreachable!(),
            };

            let frame_header = FrameHeader {
                frame_type,
                payload_len,
                mask_key,
            };

            Ok((frame_header, payload_offset))
        }
    }

    pub const fn serialized_len(&self) -> usize {
        let payload_len_len = if self.payload_len >= 65536 {
            8
        } else if self.payload_len >= 126 {
            2
        } else {
            0
        };

        2 + if self.mask_key.is_some() { 4 } else { 0 } + payload_len_len
    }

    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, Error<()>> {
        if buf.len() < self.serialized_len() {
            return Err(Error::InvalidLen);
        }

        buf[0] = 0;
        buf[1] = 0;

        if self.frame_type.is_final() {
            buf[0] |= 0x80;
        }

        let opcode = match self.frame_type {
            FrameType::Text(_) => 1,
            FrameType::Binary(_) => 2,
            FrameType::Close => 8,
            FrameType::Ping => 9,
            FrameType::Pong => 10,
            _ => 0,
        };

        buf[0] |= opcode;

        let mut payload_offset = 2;

        if self.payload_len < 126 {
            buf[1] |= self.payload_len as u8;
        } else {
            let payload_len_bytes = self.payload_len.to_be_bytes();
            if self.payload_len >= 126 && self.payload_len < 65536 {
                buf[1] |= 126;
                buf[2] = payload_len_bytes[6];
                buf[3] = payload_len_bytes[7];

                payload_offset += 2;
            } else {
                buf[1] |= 127;
                buf[2] = payload_len_bytes[0];
                buf[3] = payload_len_bytes[1];
                buf[4] = payload_len_bytes[2];
                buf[5] = payload_len_bytes[3];
                buf[6] = payload_len_bytes[4];
                buf[7] = payload_len_bytes[5];
                buf[8] = payload_len_bytes[6];
                buf[9] = payload_len_bytes[7];

                payload_offset += 8;
            }
        }

        if let Some(mask_key) = self.mask_key {
            buf[1] |= 0x80;

            let mask_key_bytes = mask_key.to_be_bytes();

            buf[payload_offset] = mask_key_bytes[0];
            buf[payload_offset + 1] = mask_key_bytes[1];
            buf[payload_offset + 2] = mask_key_bytes[2];
            buf[payload_offset + 3] = mask_key_bytes[3];

            payload_offset += 4;
        }

        Ok(payload_offset)
    }

    pub fn mask(&self, buf: &mut [u8], payload_offset: usize) {
        Self::mask_with(buf, self.mask_key, payload_offset)
    }

    pub fn mask_with(buf: &mut [u8], mask_key: Option<u32>, payload_offset: usize) {
        if let Some(mask_key) = mask_key {
            let mask_bytes = mask_key.to_be_bytes();

            for (offset, byte) in buf.iter_mut().enumerate() {
                *byte ^= mask_bytes[(payload_offset + offset) % 4];
            }
        }
    }
}

impl core::fmt::Display for FrameHeader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Frame {{ {}, payload len {}, mask {:?} }}",
            self.frame_type, self.payload_len, self.mask_key
        )
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for FrameHeader {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(
            f,
            "Frame {{ {}, payload len {}, mask {:?} }}",
            self.frame_type,
            self.payload_len,
            self.mask_key
        )
    }
}

#[cfg(test)]
mod test {
    use super::{Error, FrameHeader, FrameType};

    fn roundtrip(
        frame_type: FrameType,
        payload_len: u64,
        mask_key: Option<u32>,
        expected_len: usize,
    ) {
        let header = FrameHeader {
            frame_type,
            payload_len,
            mask_key,
        };
        assert_eq!(header.serialized_len(), expected_len);

        let mut buf = [0xaa_u8; FrameHeader::MAX_LEN];
        assert_eq!(header.serialize(&mut buf), Ok(expected_len));

        let (decoded, offset) = FrameHeader::deserialize(&buf[..expected_len]).unwrap();
        assert_eq!(offset, expected_len);
        assert_eq!(decoded.frame_type, frame_type);
        assert_eq!(decoded.payload_len, payload_len);
        assert_eq!(decoded.mask_key, mask_key);

        // Trailing payload bytes do not affect the header
        let (decoded, offset) = FrameHeader::deserialize(&buf).unwrap();
        assert_eq!(offset, expected_len);
        assert_eq!(decoded.payload_len, payload_len);
    }

    #[test]
    fn header_roundtrips() {
        roundtrip(FrameType::Text(false), 0, None, 2);
        roundtrip(FrameType::Text(true), 125, None, 2);
        roundtrip(FrameType::Binary(true), 125, Some(0xdead_beef), 6);
        roundtrip(FrameType::Binary(false), 126, None, 4);
        roundtrip(FrameType::Ping, 300, Some(1), 8);
        roundtrip(FrameType::Pong, 65535, None, 4);
        roundtrip(FrameType::Close, 65536, None, 10);
        roundtrip(FrameType::Continue(true), 1 << 40, Some(u32::MAX), 14);
        roundtrip(FrameType::Continue(false), 5, None, 2);
    }

    #[test]
    fn wire_format() {
        // A final, unmasked text frame with a 5-byte payload
        let header = FrameHeader {
            frame_type: FrameType::Text(false),
            payload_len: 5,
            mask_key: None,
        };
        let mut buf = [0u8; 2];
        assert_eq!(header.serialize(&mut buf), Ok(2));
        assert_eq!(buf, [0x81, 0x05]);

        // A fragmented, masked binary frame with a 16-bit payload length
        let header = FrameHeader {
            frame_type: FrameType::Binary(true),
            payload_len: 300,
            mask_key: Some(0x0102_0304),
        };
        let mut buf = [0u8; 8];
        assert_eq!(header.serialize(&mut buf), Ok(8));
        assert_eq!(buf, [0x02, 0x80 | 126, 0x01, 0x2c, 1, 2, 3, 4]);

        // A close frame with a 64-bit payload length
        let header = FrameHeader {
            frame_type: FrameType::Close,
            payload_len: 0x0001_0000_0000,
            mask_key: None,
        };
        let mut buf = [0u8; 10];
        assert_eq!(header.serialize(&mut buf), Ok(10));
        assert_eq!(buf, [0x88, 127, 0, 0, 0, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn deserialize_errors() {
        assert!(matches!(
            FrameHeader::deserialize(&[]),
            Err(Error::Incomplete(2))
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x81]),
            Err(Error::Incomplete(1))
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x81, 126, 0]),
            Err(Error::Incomplete(1))
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x81, 127, 0, 0]),
            Err(Error::Incomplete(6))
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x81, 0x85, 1, 2]),
            Err(Error::Incomplete(2))
        ));

        // Reserved bits and reserved opcodes are rejected
        assert!(matches!(
            FrameHeader::deserialize(&[0xc1, 0]),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x83, 0]),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            FrameHeader::deserialize(&[0x8b, 0]),
            Err(Error::Invalid)
        ));

        // Serialization needs a buffer of at least `serialized_len` bytes
        let header = FrameHeader {
            frame_type: FrameType::Binary(true),
            payload_len: 300,
            mask_key: Some(1),
        };
        assert_eq!(header.serialize(&mut [0u8; 7]), Err(Error::InvalidLen));
    }

    #[test]
    fn masking() {
        let mut data = *b"hello world";

        FrameHeader::mask_with(&mut data, Some(0x0102_0304), 0);
        assert_eq!(data[0], b'h' ^ 1);
        assert_eq!(data[1], b'e' ^ 2);
        assert_eq!(data[3], b'l' ^ 4);
        assert_eq!(data[4], b'o' ^ 1);

        // Masking is symmetric
        FrameHeader::mask_with(&mut data, Some(0x0102_0304), 0);
        assert_eq!(&data, b"hello world");

        // The payload offset selects the starting mask byte
        FrameHeader::mask_with(&mut data[1..], Some(0x0102_0304), 1);
        assert_eq!(data[0], b'h');
        assert_eq!(data[1], b'e' ^ 2);
        FrameHeader::mask_with(&mut data[1..], Some(0x0102_0304), 1);
        assert_eq!(&data, b"hello world");

        // Without a key, masking is a no-op
        FrameHeader::mask_with(&mut data, None, 0);
        assert_eq!(&data, b"hello world");

        // `mask` uses the key from the header
        let header = FrameHeader {
            frame_type: FrameType::Text(false),
            payload_len: 11,
            mask_key: Some(0xff00_0000),
        };
        header.mask(&mut data, 0);
        assert_eq!(data[0], b'h' ^ 0xff);
        assert_eq!(data[1], b'e');
    }

    #[test]
    fn frame_type_flags() {
        assert!(!FrameType::Text(false).is_fragmented());
        assert!(FrameType::Text(false).is_final());
        assert!(FrameType::Binary(true).is_fragmented());
        assert!(!FrameType::Binary(true).is_final());
        assert!(FrameType::Continue(false).is_fragmented());
        assert!(!FrameType::Continue(false).is_final());
        assert!(FrameType::Continue(true).is_fragmented());
        assert!(FrameType::Continue(true).is_final());

        for frame_type in [FrameType::Ping, FrameType::Pong, FrameType::Close] {
            assert!(!frame_type.is_fragmented());
            assert!(frame_type.is_final());
        }

        assert_eq!(FrameHeader::MIN_LEN, 2);
        assert_eq!(FrameHeader::MAX_LEN, 14);
    }
}
