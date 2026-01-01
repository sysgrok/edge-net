use core::cmp::min;

use embedded_io_async::{self, Read, ReadExactError, Write};

use super::*;

pub type Error<E> = super::Error<E>;

impl<E> Error<E>
where
    E: embedded_io_async::Error,
{
    pub fn erase(&self) -> Error<embedded_io_async::ErrorKind> {
        match self {
            Self::Incomplete(size) => Error::Incomplete(*size),
            Self::Invalid => Error::Invalid,
            Self::BufferOverflow => Error::BufferOverflow,
            Self::InvalidLen => Error::InvalidLen,
            Self::Io(e) => Error::Io(e.kind()),
        }
    }
}

impl<E> From<ReadExactError<E>> for Error<E> {
    fn from(e: ReadExactError<E>) -> Self {
        match e {
            ReadExactError::UnexpectedEof => Error::Invalid,
            ReadExactError::Other(e) => Error::Io(e),
        }
    }
}

impl FrameHeader {
    pub async fn recv<R>(mut read: R) -> Result<Self, Error<R::Error>>
    where
        R: Read,
    {
        let mut header_buf = [0; FrameHeader::MAX_LEN];
        let mut read_offset = 0;
        let mut read_end = FrameHeader::MIN_LEN;

        loop {
            read.read_exact(&mut header_buf[read_offset..read_end])
                .await
                .map_err(Error::from)?;

            match FrameHeader::deserialize(&header_buf[..read_end]) {
                Ok((header, _)) => return Ok(header),
                Err(Error::Incomplete(more)) => {
                    read_offset = read_end;
                    read_end += more;
                }
                Err(e) => return Err(e.recast()),
            }
        }
    }

    pub async fn send<W>(&self, mut write: W) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        let mut header_buf = [0; FrameHeader::MAX_LEN];
        let header_len = unwrap!(self.serialize(&mut header_buf));

        write
            .write_all(&header_buf[..header_len])
            .await
            .map_err(Error::Io)
    }

    pub async fn recv_payload<'a, R>(
        &self,
        mut read: R,
        payload_buf: &'a mut [u8],
    ) -> Result<&'a [u8], Error<R::Error>>
    where
        R: Read,
    {
        if (payload_buf.len() as u64) < self.payload_len {
            Err(Error::BufferOverflow)
        } else if self.payload_len == 0 {
            Ok(&[])
        } else {
            let payload = &mut payload_buf[..self.payload_len as _];

            read.read_exact(payload).await.map_err(Error::from)?;

            self.mask(payload, 0);

            Ok(payload)
        }
    }

    pub async fn send_payload<'a, W>(
        &'a self,
        mut write: W,
        payload: &'a [u8],
    ) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        let payload_buf_len = payload.len() as u64;

        if payload_buf_len != self.payload_len {
            Err(Error::InvalidLen)
        } else if payload.is_empty() {
            Ok(())
        } else if self.mask_key.is_none() {
            write.write_all(payload).await.map_err(Error::Io)
        } else {
            let mut buf = [0_u8; 32];

            let mut offset = 0;

            while offset < payload.len() {
                let len = min(buf.len(), payload.len() - offset);

                let buf = &mut buf[..len];

                buf.copy_from_slice(&payload[offset..offset + len]);

                self.mask(buf, offset);

                write.write_all(buf).await.map_err(Error::Io)?;

                offset += len;
            }

            Ok(())
        }
    }
}

pub async fn recv<R>(
    mut read: R,
    frame_data_buf: &mut [u8],
) -> Result<(FrameType, usize), Error<R::Error>>
where
    R: Read,
{
    let header = FrameHeader::recv(&mut read).await?;
    header.recv_payload(read, frame_data_buf).await?;

    Ok((header.frame_type, header.payload_len as _))
}

pub async fn send<W>(
    mut write: W,
    frame_type: FrameType,
    mask_key: Option<u32>,
    frame_data_buf: &[u8],
) -> Result<(), Error<W::Error>>
where
    W: Write,
{
    let header = FrameHeader {
        frame_type,
        payload_len: frame_data_buf.len() as _,
        mask_key,
    };

    header.send(&mut write).await?;
    header.send_payload(write, frame_data_buf).await
}
