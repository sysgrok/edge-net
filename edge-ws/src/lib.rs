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
