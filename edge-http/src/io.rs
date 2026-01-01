use core::cmp::min;
use core::fmt::{Display, Write as _};
use core::str;

use embedded_io_async::{ErrorType, Read, Write};

use httparse::Status;

use crate::ws::UpgradeError;
use crate::{
    BodyType, ConnectionType, Headers, HeadersMismatchError, Method, RequestHeaders,
    ResponseHeaders,
};

pub mod client;
pub mod server;

/// An error in parsing the headers or the body.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Error<E> {
    InvalidHeaders,
    InvalidBody,
    TooManyHeaders,
    TooLongHeaders,
    TooLongBody,
    IncompleteHeaders,
    IncompleteBody,
    InvalidState,
    ConnectionClosed,
    HeadersMismatchError(HeadersMismatchError),
    WsUpgradeError(UpgradeError),
    Io(E),
}

pub type ErrorKind = Error<edge_nal::io::ErrorKind>;

impl<E> Error<E>
where
    E: edge_nal::io::Error,
{
    pub fn erase(&self) -> Error<edge_nal::io::ErrorKind> {
        match self {
            Self::InvalidHeaders => Error::InvalidHeaders,
            Self::InvalidBody => Error::InvalidBody,
            Self::TooManyHeaders => Error::TooManyHeaders,
            Self::TooLongHeaders => Error::TooLongHeaders,
            Self::TooLongBody => Error::TooLongBody,
            Self::IncompleteHeaders => Error::IncompleteHeaders,
            Self::IncompleteBody => Error::IncompleteBody,
            Self::InvalidState => Error::InvalidState,
            Self::ConnectionClosed => Error::ConnectionClosed,
            Self::HeadersMismatchError(e) => Error::HeadersMismatchError(*e),
            Self::WsUpgradeError(e) => Error::WsUpgradeError(*e),
            Self::Io(e) => Error::Io(e.kind()),
        }
    }
}

impl<E> From<httparse::Error> for Error<E> {
    fn from(e: httparse::Error) -> Self {
        match e {
            httparse::Error::HeaderName => Self::InvalidHeaders,
            httparse::Error::HeaderValue => Self::InvalidHeaders,
            httparse::Error::NewLine => Self::InvalidHeaders,
            httparse::Error::Status => Self::InvalidHeaders,
            httparse::Error::Token => Self::InvalidHeaders,
            httparse::Error::TooManyHeaders => Self::TooManyHeaders,
            httparse::Error::Version => Self::InvalidHeaders,
        }
    }
}

impl<E> From<HeadersMismatchError> for Error<E> {
    fn from(e: HeadersMismatchError) -> Self {
        Self::HeadersMismatchError(e)
    }
}

impl<E> From<UpgradeError> for Error<E> {
    fn from(e: UpgradeError) -> Self {
        Self::WsUpgradeError(e)
    }
}

impl<E> embedded_io_async::Error for Error<E>
where
    E: embedded_io_async::Error,
{
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            Self::Io(e) => e.kind(),
            _ => embedded_io_async::ErrorKind::Other,
        }
    }
}

impl<E> Display for Error<E>
where
    E: Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidHeaders => write!(f, "Invalid HTTP headers or status line"),
            Self::InvalidBody => write!(f, "Invalid HTTP body"),
            Self::TooManyHeaders => write!(f, "Too many HTTP headers"),
            Self::TooLongHeaders => write!(f, "HTTP headers section is too long"),
            Self::TooLongBody => write!(f, "HTTP body is too long"),
            Self::IncompleteHeaders => write!(f, "HTTP headers section is incomplete"),
            Self::IncompleteBody => write!(f, "HTTP body is incomplete"),
            Self::InvalidState => write!(f, "Connection is not in requested state"),
            Self::HeadersMismatchError(e) => write!(f, "Headers mismatch: {e}"),
            Self::WsUpgradeError(e) => write!(f, "WebSocket upgrade error: {e}"),
            Self::ConnectionClosed => write!(f, "Connection closed"),
            Self::Io(e) => write!(f, "{e}"),
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
            Self::InvalidHeaders => defmt::write!(f, "Invalid HTTP headers or status line"),
            Self::InvalidBody => defmt::write!(f, "Invalid HTTP body"),
            Self::TooManyHeaders => defmt::write!(f, "Too many HTTP headers"),
            Self::TooLongHeaders => defmt::write!(f, "HTTP headers section is too long"),
            Self::TooLongBody => defmt::write!(f, "HTTP body is too long"),
            Self::IncompleteHeaders => defmt::write!(f, "HTTP headers section is incomplete"),
            Self::IncompleteBody => defmt::write!(f, "HTTP body is incomplete"),
            Self::InvalidState => defmt::write!(f, "Connection is not in requested state"),
            Self::HeadersMismatchError(e) => defmt::write!(f, "Headers mismatch: {}", e),
            Self::WsUpgradeError(e) => defmt::write!(f, "WebSocket upgrade error: {}", e),
            Self::ConnectionClosed => defmt::write!(f, "Connection closed"),
            Self::Io(e) => defmt::write!(f, "{}", e),
        }
    }
}

impl<E> core::error::Error for Error<E> where E: core::error::Error {}

impl<'b, const N: usize> RequestHeaders<'b, N> {
    /// Parse the headers from the input stream
    pub async fn receive<R>(
        &mut self,
        buf: &'b mut [u8],
        mut input: R,
        exact: bool,
    ) -> Result<(&'b mut [u8], usize), Error<R::Error>>
    where
        R: Read,
    {
        let (read_len, headers_len) =
            match raw::read_reply_buf::<N, _>(&mut input, buf, true, exact).await {
                Ok(read_len) => read_len,
                Err(e) => return Err(e),
            };

        let mut parser = httparse::Request::new(&mut self.headers.0);

        let (headers_buf, body_buf) = buf.split_at_mut(headers_len);

        let status = match parser.parse(headers_buf) {
            Ok(status) => status,
            Err(e) => return Err(e.into()),
        };

        if let Status::Complete(headers_len2) = status {
            if headers_len != headers_len2 {
                unreachable!("Should not happen. HTTP header parsing is indeterminate.")
            }

            self.http11 = match parser.version {
                Some(0) => false,
                Some(1) => true,
                _ => Err(Error::InvalidHeaders)?,
            };

            let method_str = parser.method.ok_or(Error::InvalidHeaders)?;
            self.method = Method::new(method_str).ok_or(Error::InvalidHeaders)?;
            self.path = parser.path.ok_or(Error::InvalidHeaders)?;

            trace!("Received:\n{}", self);

            Ok((body_buf, read_len - headers_len))
        } else {
            unreachable!("Secondary parse of already loaded buffer failed.")
        }
    }

    /// Resolve the connection type and body type from the headers
    pub fn resolve<E>(&self) -> Result<(ConnectionType, BodyType), Error<E>> {
        self.headers.resolve::<E>(None, true, self.http11)
    }

    /// Send the headers to the output stream, returning the connection type and body type
    pub async fn send<W>(
        &self,
        chunked_if_unspecified: bool,
        mut output: W,
    ) -> Result<(ConnectionType, BodyType), Error<W::Error>>
    where
        W: Write,
    {
        send_request(self.http11, self.method, self.path, &mut output).await?;

        self.headers
            .send(None, true, self.http11, chunked_if_unspecified, output)
            .await
    }
}

impl<'b, const N: usize> ResponseHeaders<'b, N> {
    /// Parse the headers from the input stream
    pub async fn receive<R>(
        &mut self,
        buf: &'b mut [u8],
        mut input: R,
        exact: bool,
    ) -> Result<(&'b mut [u8], usize), Error<R::Error>>
    where
        R: Read,
    {
        let (read_len, headers_len) =
            raw::read_reply_buf::<N, _>(&mut input, buf, false, exact).await?;

        let mut parser = httparse::Response::new(&mut self.headers.0);

        let (headers_buf, body_buf) = buf.split_at_mut(headers_len);

        let status = parser.parse(headers_buf).map_err(Error::from)?;

        if let Status::Complete(headers_len2) = status {
            if headers_len != headers_len2 {
                unreachable!("Should not happen. HTTP header parsing is indeterminate.")
            }

            self.http11 = match parser.version {
                Some(0) => false,
                Some(1) => true,
                _ => Err(Error::InvalidHeaders)?,
            };

            self.code = parser.code.ok_or(Error::InvalidHeaders)?;
            self.reason = parser.reason;

            trace!("Received:\n{}", self);

            Ok((body_buf, read_len - headers_len))
        } else {
            unreachable!("Secondary parse of already loaded buffer failed.")
        }
    }

    /// Resolve the connection type and body type from the headers
    pub fn resolve<E>(
        &self,
        request_connection_type: ConnectionType,
    ) -> Result<(ConnectionType, BodyType), Error<E>> {
        self.headers
            .resolve::<E>(Some(request_connection_type), false, self.http11)
    }

    /// Send the headers to the output stream, returning the connection type and body type
    pub async fn send<W>(
        &self,
        request_connection_type: ConnectionType,
        chunked_if_unspecified: bool,
        mut output: W,
    ) -> Result<(ConnectionType, BodyType), Error<W::Error>>
    where
        W: Write,
    {
        send_status(self.http11, self.code, self.reason, &mut output).await?;

        self.headers
            .send(
                Some(request_connection_type),
                false,
                self.http11,
                chunked_if_unspecified,
                output,
            )
            .await
    }
}

pub(crate) async fn send_request<W>(
    http11: bool,
    method: Method,
    path: &str,
    mut output: W,
) -> Result<(), Error<W::Error>>
where
    W: Write,
{
    // RFC 9112:   request-line   = method SP request-target SP HTTP-version

    output
        .write_all(method.as_str().as_bytes())
        .await
        .map_err(Error::Io)?;
    output.write_all(b" ").await.map_err(Error::Io)?;
    output.write_all(path.as_bytes()).await.map_err(Error::Io)?;
    output.write_all(b" ").await.map_err(Error::Io)?;
    raw::send_version(&mut output, http11).await?;
    output.write_all(b"\r\n").await.map_err(Error::Io)?;

    Ok(())
}

pub(crate) async fn send_status<W>(
    http11: bool,
    status: u16,
    reason: Option<&str>,
    mut output: W,
) -> Result<(), Error<W::Error>>
where
    W: Write,
{
    // RFC 9112:   status-line = HTTP-version SP status-code SP [ reason-phrase ]

    raw::send_version(&mut output, http11).await?;
    output.write_all(b" ").await.map_err(Error::Io)?;
    let status_str: heapless::String<5> = unwrap!(status.try_into());
    output
        .write_all(status_str.as_bytes())
        .await
        .map_err(Error::Io)?;
    output.write_all(b" ").await.map_err(Error::Io)?;
    if let Some(reason) = reason {
        output
            .write_all(reason.as_bytes())
            .await
            .map_err(Error::Io)?;
    }
    output.write_all(b"\r\n").await.map_err(Error::Io)?;

    Ok(())
}

pub(crate) async fn send_headers<'a, H, W>(
    headers: H,
    carry_over_connection_type: Option<ConnectionType>,
    request: bool,
    http11: bool,
    chunked_if_unspecified: bool,
    mut output: W,
) -> Result<(ConnectionType, BodyType), Error<W::Error>>
where
    W: Write,
    H: IntoIterator<Item = &'a (&'a str, &'a str)>,
{
    let (headers_connection_type, headers_body_type) = raw::send_headers(
        headers
            .into_iter()
            .map(|(name, value)| (*name, value.as_bytes())),
        &mut output,
    )
    .await?;

    send_headers_end(
        headers_connection_type,
        headers_body_type,
        carry_over_connection_type,
        request,
        http11,
        chunked_if_unspecified,
        output,
    )
    .await
}

async fn send_headers_end<W>(
    headers_connection_type: Option<ConnectionType>,
    headers_body_type: Option<BodyType>,
    carry_over_connection_type: Option<ConnectionType>,
    request: bool,
    http11: bool,
    chunked_if_unspecified: bool,
    mut output: W,
) -> Result<(ConnectionType, BodyType), Error<W::Error>>
where
    W: Write,
{
    let connection_type =
        ConnectionType::resolve(headers_connection_type, carry_over_connection_type, http11)?;

    let body_type = BodyType::resolve(
        headers_body_type,
        connection_type,
        request,
        http11,
        chunked_if_unspecified,
    )?;

    if headers_connection_type.is_none() {
        // Send an explicit Connection-Type just in case
        let (name, value) = connection_type.raw_header();

        raw::send_header(name, value, &mut output).await?;
    }

    if headers_body_type.is_none() {
        let mut buf = heapless::String::new();

        if let Some((name, value)) = body_type.raw_header(&mut buf) {
            // Send explicit body type header just in case or if the body type was upgraded
            raw::send_header(name, value, &mut output).await?;
        }
    }

    raw::send_headers_end(output).await?;

    Ok((connection_type, body_type))
}

impl<const N: usize> Headers<'_, N> {
    fn resolve<E>(
        &self,
        carry_over_connection_type: Option<ConnectionType>,
        request: bool,
        http11: bool,
    ) -> Result<(ConnectionType, BodyType), Error<E>> {
        let headers_connection_type = ConnectionType::from_headers(self.iter());
        let headers_body_type = BodyType::from_headers(self.iter());

        let connection_type =
            ConnectionType::resolve(headers_connection_type, carry_over_connection_type, http11)?;
        let body_type =
            BodyType::resolve(headers_body_type, connection_type, request, http11, false)?;

        Ok((connection_type, body_type))
    }

    async fn send<W>(
        &self,
        carry_over_connection_type: Option<ConnectionType>,
        request: bool,
        http11: bool,
        chunked_if_unspecified: bool,
        mut output: W,
    ) -> Result<(ConnectionType, BodyType), Error<W::Error>>
    where
        W: Write,
    {
        let (headers_connection_type, headers_body_type) =
            raw::send_headers(self.iter_raw(), &mut output).await?;

        send_headers_end(
            headers_connection_type,
            headers_body_type,
            carry_over_connection_type,
            request,
            http11,
            chunked_if_unspecified,
            output,
        )
        .await
    }
}

/// Represents an incoming HTTP request stream body
///
/// Implements the `Read` trait to read the body from the stream
#[allow(private_interfaces)]
pub enum Body<'b, R> {
    /// The body is raw and should be read as is (only possible for HTTP responses with connection = Close)
    Raw(PartiallyRead<'b, R>),
    /// The body is of a known length (Content-Length)
    ContentLen(ContentLenRead<PartiallyRead<'b, R>>),
    /// The body is chunked (Transfer-Encoding: chunked)
    Chunked(ChunkedRead<'b, PartiallyRead<'b, R>>),
}

impl<'b, R> Body<'b, R>
where
    R: Read,
{
    /// Create a new body
    ///
    /// Parameters:
    /// - `body_type`: The type of the body, as resolved using `BodyType::resolve`
    /// - `buf`: The buffer to use for reading the body
    /// - `read_len`: The length of the buffer that has already been read when processing the icoming headers
    /// - `input`: The raw input stream
    pub fn new(body_type: BodyType, buf: &'b mut [u8], read_len: usize, input: R) -> Self {
        match body_type {
            BodyType::Chunked => Body::Chunked(ChunkedRead::new(
                PartiallyRead::new(&[], input),
                buf,
                read_len,
            )),
            BodyType::ContentLen(content_len) => Body::ContentLen(ContentLenRead::new(
                content_len,
                PartiallyRead::new(&buf[..read_len], input),
            )),
            BodyType::Raw => Body::Raw(PartiallyRead::new(&buf[..read_len], input)),
        }
    }

    /// Check if the body needs to be closed (i.e. the underlying input stream cannot be re-used for Keep-Alive connections)
    pub fn needs_close(&self) -> bool {
        !self.is_complete() || matches!(self, Self::Raw(_))
    }

    /// Check if the body has been completely read
    pub fn is_complete(&self) -> bool {
        match self {
            Self::Raw(_) => true,
            Self::ContentLen(r) => r.is_complete(),
            Self::Chunked(r) => r.is_complete(),
        }
    }

    /// Return a mutable reference to the underlying raw reader
    pub fn as_raw_reader(&mut self) -> &mut R {
        match self {
            Self::Raw(r) => &mut r.input,
            Self::ContentLen(r) => &mut r.input.input,
            Self::Chunked(r) => &mut r.input.input,
        }
    }

    /// Release the body, returning the underlying raw reader
    pub fn release(self) -> R {
        match self {
            Self::Raw(r) => r.release(),
            Self::ContentLen(r) => r.release().release(),
            Self::Chunked(r) => r.release().release(),
        }
    }
}

impl<R> ErrorType for Body<'_, R>
where
    R: ErrorType,
{
    type Error = Error<R::Error>;
}

impl<R> Read for Body<'_, R>
where
    R: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Raw(read) => Ok(read.read(buf).await.map_err(Error::Io)?),
            Self::ContentLen(read) => Ok(read.read(buf).await?),
            Self::Chunked(read) => Ok(read.read(buf).await?),
        }
    }
}

pub(crate) struct PartiallyRead<'b, R> {
    buf: &'b [u8],
    read_len: usize,
    input: R,
}

impl<'b, R> PartiallyRead<'b, R> {
    pub const fn new(buf: &'b [u8], input: R) -> Self {
        Self {
            buf,
            read_len: 0,
            input,
        }
    }

    // pub fn buf_len(&self) -> usize {
    //     self.buf.len()
    // }

    // pub fn as_raw_reader(&mut self) -> &mut R {
    //     &mut self.input
    // }

    pub fn release(self) -> R {
        self.input
    }
}

impl<R> ErrorType for PartiallyRead<'_, R>
where
    R: ErrorType,
{
    type Error = R::Error;
}

impl<R> Read for PartiallyRead<'_, R>
where
    R: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if self.buf.len() > self.read_len {
            let len = min(buf.len(), self.buf.len() - self.read_len);
            buf[..len].copy_from_slice(&self.buf[self.read_len..self.read_len + len]);

            self.read_len += len;

            Ok(len)
        } else {
            Ok(self.input.read(buf).await?)
        }
    }
}

pub(crate) struct ContentLenRead<R> {
    content_len: u64,
    read_len: u64,
    input: R,
}

impl<R> ContentLenRead<R> {
    pub const fn new(content_len: u64, input: R) -> Self {
        Self {
            content_len,
            read_len: 0,
            input,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.content_len == self.read_len
    }

    pub fn release(self) -> R {
        self.input
    }
}

impl<R> ErrorType for ContentLenRead<R>
where
    R: ErrorType,
{
    type Error = Error<R::Error>;
}

impl<R> Read for ContentLenRead<R>
where
    R: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let len = min(buf.len() as _, self.content_len - self.read_len);
        if len > 0 {
            let read = self
                .input
                .read(&mut buf[..len as _])
                .await
                .map_err(Error::Io)?;
            self.read_len += read as u64;

            Ok(read)
        } else {
            Ok(0)
        }
    }
}

pub(crate) struct ChunkedRead<'b, R> {
    buf: &'b mut [u8],
    buf_offset: usize,
    buf_len: usize,
    input: R,
    remain: u64,
    complete: bool,
}

impl<'b, R> ChunkedRead<'b, R>
where
    R: Read,
{
    pub fn new(input: R, buf: &'b mut [u8], buf_len: usize) -> Self {
        Self {
            buf,
            buf_offset: 0,
            buf_len,
            input,
            remain: 0,
            complete: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn release(self) -> R {
        self.input
    }

    // The elegant pull parser taken from here:
    // https://github.com/kchmck/uhttp_chunked_bytes.rs/blob/master/src/lib.rs
    // Changes:
    // - Converted to async
    // - Iterators removed
    // - Simpler error handling
    // - Consumption of trailer
    async fn next(&mut self) -> Result<Option<u8>, Error<R::Error>> {
        if self.complete {
            return Ok(None);
        }

        if self.remain == 0 {
            if let Some(size) = self.parse_size().await? {
                // If chunk size is zero (final chunk), the stream is finished [RFC7230§4.1].
                if size == 0 {
                    self.consume_trailer().await?;
                    self.complete = true;
                    return Ok(None);
                }

                self.remain = size;
            } else {
                self.complete = true;
                return Ok(None);
            }
        }

        let next = self.input_fetch().await?;
        self.remain -= 1;

        // If current chunk is finished, verify it ends with CRLF [RFC7230§4.1].
        if self.remain == 0 {
            self.consume_multi(b"\r\n").await?;
        }

        Ok(Some(next))
    }

    // Parse the number of bytes in the next chunk.
    async fn parse_size(&mut self) -> Result<Option<u64>, Error<R::Error>> {
        let mut digits = [0_u8; 16];

        let slice = match self.parse_digits(&mut digits[..]).await? {
            Some(s) => str::from_utf8(s).map_err(|_| Error::InvalidBody)?,
            None => return Ok(None),
        };

        let size = u64::from_str_radix(slice, 16).map_err(|_| Error::InvalidBody)?;

        Ok(Some(size))
    }

    // Extract the hex digits for the current chunk size.
    async fn parse_digits<'a>(
        &'a mut self,
        digits: &'a mut [u8],
    ) -> Result<Option<&'a [u8]>, Error<R::Error>> {
        // Number of hex digits that have been extracted.
        let mut len = 0;

        loop {
            let b = match self.input_next().await? {
                Some(b) => b,
                None => {
                    return if len == 0 {
                        // If EOF at the beginning of a new chunk, the stream is finished.
                        Ok(None)
                    } else {
                        Err(Error::IncompleteBody)
                    };
                }
            };

            match b {
                b'\r' => {
                    self.consume(b'\n').await?;
                    break;
                }
                b';' => {
                    self.consume_ext().await?;
                    break;
                }
                _ => {
                    match digits.get_mut(len) {
                        Some(d) => *d = b,
                        None => return Err(Error::InvalidBody),
                    }

                    len += 1;
                }
            }
        }

        Ok(Some(&digits[..len]))
    }

    // Consume and discard current chunk extension.
    // This doesn't check whether the characters up to CRLF actually have correct syntax.
    async fn consume_ext(&mut self) -> Result<(), Error<R::Error>> {
        self.consume_header().await?;

        Ok(())
    }

    // Consume and discard the optional trailer following the last chunk.
    async fn consume_trailer(&mut self) -> Result<(), Error<R::Error>> {
        while self.consume_header().await? {}

        Ok(())
    }

    // Consume and discard each header in the optional trailer following the last chunk.
    async fn consume_header(&mut self) -> Result<bool, Error<R::Error>> {
        let mut first = self.input_fetch().await?;
        let mut len = 1;

        loop {
            let second = self.input_fetch().await?;
            len += 1;

            if first == b'\r' && second == b'\n' {
                return Ok(len > 2);
            }

            first = second;
        }
    }

    // Verify the next bytes in the stream match the expectation.
    async fn consume_multi(&mut self, bytes: &[u8]) -> Result<(), Error<R::Error>> {
        for byte in bytes {
            self.consume(*byte).await?;
        }

        Ok(())
    }

    // Verify the next byte in the stream is matching the expectation.
    async fn consume(&mut self, byte: u8) -> Result<(), Error<R::Error>> {
        if self.input_fetch().await? == byte {
            Ok(())
        } else {
            Err(Error::InvalidBody)
        }
    }

    async fn input_fetch(&mut self) -> Result<u8, Error<R::Error>> {
        self.input_next().await?.ok_or(Error::IncompleteBody)
    }

    async fn input_next(&mut self) -> Result<Option<u8>, Error<R::Error>> {
        if self.buf_offset == self.buf_len {
            self.buf_len = self.input.read(self.buf).await.map_err(Error::Io)?;
            self.buf_offset = 0;
        }

        if self.buf_len > 0 {
            let byte = self.buf[self.buf_offset];
            self.buf_offset += 1;

            Ok(Some(byte))
        } else {
            Ok(None)
        }
    }
}

impl<R> ErrorType for ChunkedRead<'_, R>
where
    R: ErrorType,
{
    type Error = Error<R::Error>;
}

impl<R> Read for ChunkedRead<'_, R>
where
    R: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        for (index, byte_pos) in buf.iter_mut().enumerate() {
            if let Some(byte) = self.next().await? {
                *byte_pos = byte;
            } else {
                return Ok(index);
            }
        }

        Ok(buf.len())
    }
}

/// Represents an outgoing HTTP request stream body
///
/// Implements the `Write` trait to write the body to the stream
#[allow(private_interfaces)]
pub enum SendBody<W> {
    /// The body is raw and should be written as is (only possible for HTTP responses with connection = Close)
    Raw(W),
    /// The body is of a known length (Content-Length)
    ContentLen(ContentLenWrite<W>),
    /// The body is chunked (Transfer-Encoding: chunked)
    Chunked(ChunkedWrite<W>),
}

impl<W> SendBody<W>
where
    W: Write,
{
    /// Create a new body
    ///
    /// Parameters:
    /// - `body_type`: The type of the body, as resolved using `BodyType::resolve`
    /// - `output`: The raw output stream
    pub fn new(body_type: BodyType, output: W) -> SendBody<W> {
        match body_type {
            BodyType::Chunked => SendBody::Chunked(ChunkedWrite::new(output)),
            BodyType::ContentLen(content_len) => {
                SendBody::ContentLen(ContentLenWrite::new(content_len, output))
            }
            BodyType::Raw => SendBody::Raw(output),
        }
    }

    /// Check if the body has been completely written to
    pub fn is_complete(&self) -> bool {
        match self {
            Self::ContentLen(w) => w.is_complete(),
            _ => true,
        }
    }

    /// Check if the body needs to be closed (i.e. the underlying output stream cannot be re-used for Keep-Alive connections)
    pub fn needs_close(&self) -> bool {
        !self.is_complete() || matches!(self, Self::Raw(_))
    }

    /// Finish writing the body (necessary for chunked encoding)
    pub async fn finish(&mut self) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        match self {
            Self::Raw(_) => (),
            Self::ContentLen(w) => {
                if !w.is_complete() {
                    return Err(Error::IncompleteBody);
                }
            }
            Self::Chunked(w) => w.finish().await?,
        }

        self.flush().await?;

        Ok(())
    }

    /// Return a mutable reference to the underlying raw writer
    pub fn as_raw_writer(&mut self) -> &mut W {
        match self {
            Self::Raw(w) => w,
            Self::ContentLen(w) => &mut w.output,
            Self::Chunked(w) => &mut w.output,
        }
    }

    /// Release the body, returning the underlying raw writer
    pub fn release(self) -> W {
        match self {
            Self::Raw(w) => w,
            Self::ContentLen(w) => w.release(),
            Self::Chunked(w) => w.release(),
        }
    }
}

impl<W> ErrorType for SendBody<W>
where
    W: ErrorType,
{
    type Error = Error<W::Error>;
}

impl<W> Write for SendBody<W>
where
    W: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Raw(w) => Ok(w.write(buf).await.map_err(Error::Io)?),
            Self::ContentLen(w) => Ok(w.write(buf).await?),
            Self::Chunked(w) => Ok(w.write(buf).await?),
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        match self {
            Self::Raw(w) => Ok(w.flush().await.map_err(Error::Io)?),
            Self::ContentLen(w) => Ok(w.flush().await?),
            Self::Chunked(w) => Ok(w.flush().await?),
        }
    }
}

pub(crate) struct ContentLenWrite<W> {
    content_len: u64,
    write_len: u64,
    output: W,
}

impl<W> ContentLenWrite<W> {
    pub const fn new(content_len: u64, output: W) -> Self {
        Self {
            content_len,
            write_len: 0,
            output,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.content_len == self.write_len
    }

    pub fn release(self) -> W {
        self.output
    }
}

impl<W> ErrorType for ContentLenWrite<W>
where
    W: ErrorType,
{
    type Error = Error<W::Error>;
}

impl<W> Write for ContentLenWrite<W>
where
    W: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if self.content_len >= self.write_len + buf.len() as u64 {
            let write = self.output.write(buf).await.map_err(Error::Io)?;
            self.write_len += write as u64;

            Ok(write)
        } else {
            Err(Error::TooLongBody)
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.output.flush().await.map_err(Error::Io)
    }
}

pub(crate) struct ChunkedWrite<W> {
    output: W,
    finished: bool,
}

impl<W> ChunkedWrite<W> {
    pub const fn new(output: W) -> Self {
        Self {
            output,
            finished: false,
        }
    }

    pub async fn finish(&mut self) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        if !self.finished {
            self.output
                .write_all(b"0\r\n\r\n")
                .await
                .map_err(Error::Io)?;
            self.finished = true;
        }

        Ok(())
    }

    pub fn release(self) -> W {
        self.output
    }
}

impl<W> ErrorType for ChunkedWrite<W>
where
    W: ErrorType,
{
    type Error = Error<W::Error>;
}

impl<W> Write for ChunkedWrite<W>
where
    W: Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if self.finished {
            Err(Error::InvalidState)
        } else if !buf.is_empty() {
            let mut len_str = heapless::String::<8>::new();
            write_unwrap!(&mut len_str, "{:x}", buf.len());

            self.output
                .write_all(len_str.as_bytes())
                .await
                .map_err(Error::Io)?;

            self.output.write_all(b"\r\n").await.map_err(Error::Io)?;
            self.output.write_all(buf).await.map_err(Error::Io)?;
            self.output.write_all(b"\r\n").await.map_err(Error::Io)?;

            Ok(buf.len())
        } else {
            Ok(0)
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.output.flush().await.map_err(Error::Io)
    }
}

mod raw {
    use core::str;

    use embedded_io_async::{Read, Write};

    use crate::{BodyType, ConnectionType};

    use super::Error;

    pub(crate) async fn read_reply_buf<const N: usize, R>(
        mut input: R,
        buf: &mut [u8],
        request: bool,
        exact: bool,
    ) -> Result<(usize, usize), Error<R::Error>>
    where
        R: Read,
    {
        if exact {
            let raw_headers_len = read_headers(&mut input, buf).await?;

            let mut headers = [httparse::EMPTY_HEADER; N];

            let status = if request {
                httparse::Request::new(&mut headers).parse(&buf[..raw_headers_len])?
            } else {
                httparse::Response::new(&mut headers).parse(&buf[..raw_headers_len])?
            };

            if let httparse::Status::Complete(headers_len) = status {
                return Ok((raw_headers_len, headers_len));
            }

            Err(Error::TooManyHeaders)
        } else {
            let mut offset = 0;
            let mut size = 0;

            while buf.len() > size {
                let read = input.read(&mut buf[offset..]).await.map_err(Error::Io)?;
                if read == 0 {
                    Err(if offset == 0 {
                        Error::ConnectionClosed
                    } else {
                        Error::IncompleteHeaders
                    })?;
                }

                offset += read;
                size += read;

                let mut headers = [httparse::EMPTY_HEADER; N];

                let status = if request {
                    httparse::Request::new(&mut headers).parse(&buf[..size])?
                } else {
                    httparse::Response::new(&mut headers).parse(&buf[..size])?
                };

                if let httparse::Status::Complete(headers_len) = status {
                    return Ok((size, headers_len));
                }
            }

            Err(Error::TooManyHeaders)
        }
    }

    pub(crate) async fn read_headers<R>(
        mut input: R,
        buf: &mut [u8],
    ) -> Result<usize, Error<R::Error>>
    where
        R: Read,
    {
        let mut offset = 0;
        let mut byte = [0];

        loop {
            if offset == buf.len() {
                Err(Error::TooLongHeaders)?;
            }

            let read = input.read(&mut byte).await.map_err(Error::Io)?;

            if read == 0 {
                Err(if offset == 0 {
                    Error::ConnectionClosed
                } else {
                    Error::IncompleteHeaders
                })?;
            }

            buf[offset] = byte[0];

            offset += 1;

            if offset >= b"\r\n\r\n".len() && buf[offset - 4..offset] == *b"\r\n\r\n" {
                break Ok(offset);
            }
        }
    }

    pub(crate) async fn send_version<W>(mut output: W, http11: bool) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        output
            .write_all(if http11 { b"HTTP/1.1" } else { b"HTTP/1.0" })
            .await
            .map_err(Error::Io)
    }

    pub(crate) async fn send_headers<'a, H, W>(
        headers: H,
        mut output: W,
    ) -> Result<(Option<ConnectionType>, Option<BodyType>), Error<W::Error>>
    where
        W: Write,
        H: IntoIterator<Item = (&'a str, &'a [u8])>,
    {
        let mut connection = None;
        let mut body = None;

        for (name, value) in headers.into_iter() {
            let header_connection =
                ConnectionType::from_header(name, str::from_utf8(value).unwrap_or(""));

            if let Some(header_connection) = header_connection {
                if let Some(connection) = connection {
                    warn!(
                        "Multiple Connection headers found. Current {} and new {}",
                        connection, header_connection
                    );
                }

                // The last connection header wins
                connection = Some(header_connection);
            }

            let header_body = BodyType::from_header(name, str::from_utf8(value).unwrap_or(""));

            if let Some(header_body) = header_body {
                if let Some(body) = body {
                    warn!(
                        "Multiple body type headers found. Current {} and new {}",
                        body, header_body
                    );
                }

                // The last body header wins
                body = Some(header_body);
            }

            send_header(name, value, &mut output).await?;
        }

        Ok((connection, body))
    }

    pub(crate) async fn send_header<W>(
        name: &str,
        value: &[u8],
        mut output: W,
    ) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        output.write_all(name.as_bytes()).await.map_err(Error::Io)?;
        output.write_all(b": ").await.map_err(Error::Io)?;
        output.write_all(value).await.map_err(Error::Io)?;
        output.write_all(b"\r\n").await.map_err(Error::Io)?;

        Ok(())
    }

    pub(crate) async fn send_headers_end<W>(mut output: W) -> Result<(), Error<W::Error>>
    where
        W: Write,
    {
        output.write_all(b"\r\n").await.map_err(Error::Io)
    }
}

#[cfg(test)]
mod test {
    use embedded_io_async::{ErrorType, Read};

    use super::*;

    struct SliceRead<'a>(&'a [u8]);

    impl<'a> ErrorType for SliceRead<'a> {
        type Error = core::convert::Infallible;
    }

    impl<'a> Read for SliceRead<'a> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let len = core::cmp::min(buf.len(), self.0.len());
            buf[..len].copy_from_slice(&self.0[..len]);

            self.0 = &self.0[len..];

            Ok(len)
        }
    }

    #[test]
    fn test_chunked_bytes() {
        // Normal
        expect(b"A\r\nabcdefghij\r\n2\r\n42\r\n", Some(b"abcdefghij42"));
        expect(b"a\r\nabc\r\nfghij\r\n2\r\n42\r\n", Some(b"abc\r\nfghij42"));

        // Trailing headers
        expect(b"4\r\nabcd\r\n0\r\n\r\n", Some(b"abcd"));
        expect(b"4\r\nabcd\r\n0\r\nA: B\r\n\r\n", Some(b"abcd"));

        // Empty
        expect(b"", Some(b""));
        expect(b"0\r\n\r\n", Some(b""));

        // Erroneous
        expect(b"h\r\n", None);
        expect(b"\r\na", None);
        expect(b"4\r\nabcdefg", None);
    }

    fn expect(input: &[u8], expected: Option<&[u8]>) {
        embassy_futures::block_on(async move {
            let mut buf1 = [0; 64];
            let mut buf2 = [0; 64];

            let stream = SliceRead(input);
            let mut r = ChunkedRead::new(stream, &mut buf1, 0);

            if let Some(expected) = expected {
                assert!(r.read_exact(&mut buf2[..expected.len()]).await.is_ok());

                assert_eq!(&buf2[..expected.len()], expected);

                let len = r.read(&mut buf2).await;
                assert!(len.is_ok());

                assert_eq!(unwrap!(len), 0);
            } else {
                assert!(r.read(&mut buf2).await.is_err());
            }
        })
    }
}
