#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::fmt::Display;
use core::str;

use httparse::{Header, EMPTY_HEADER};

use ws::{is_upgrade_accepted, is_upgrade_request, MAX_BASE64_KEY_RESPONSE_LEN, NONCE_LEN};

pub const DEFAULT_MAX_HEADERS_COUNT: usize = 64;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "io")]
pub mod io;

/// Errors related to invalid combinations of connection type
/// and body type (Content-Length, Transfer-Encoding) in the headers
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum HeadersMismatchError {
    /// Connection type mismatch: Keep-Alive connection type in the response,
    /// while the request contained a Close connection type
    ResponseConnectionTypeMismatchError,
    /// Body type mismatch: the body type in the headers cannot be used with the specified connection type and HTTP protocol.
    /// This is often a user-error, but might also come from the other peer not following the protocol.
    /// I.e.:
    /// - Chunked body with an HTTP1.0 connection
    /// - Raw body with a Keep-Alive connection
    /// - etc.
    BodyTypeError(&'static str),
}

impl Display for HeadersMismatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ResponseConnectionTypeMismatchError => write!(
                f,
                "Response connection type is different from the request connection type"
            ),
            Self::BodyTypeError(s) => write!(f, "Body type mismatch: {s}"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for HeadersMismatchError {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::ResponseConnectionTypeMismatchError => defmt::write!(
                f,
                "Response connection type is different from the request connection type"
            ),
            Self::BodyTypeError(s) => defmt::write!(f, "Body type mismatch: {}", s),
        }
    }
}

impl core::error::Error for HeadersMismatchError {}

/// Http methods
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(Hash))]
pub enum Method {
    Delete,
    Get,
    Head,
    Post,
    Put,
    Connect,
    Options,
    Trace,
    Copy,
    Lock,
    MkCol,
    Move,
    Propfind,
    Proppatch,
    Search,
    Unlock,
    Bind,
    Rebind,
    Unbind,
    Acl,
    Report,
    MkActivity,
    Checkout,
    Merge,
    MSearch,
    Notify,
    Subscribe,
    Unsubscribe,
    Patch,
    Purge,
    MkCalendar,
    Link,
    Unlink,
}

impl Method {
    pub fn new(method: &str) -> Option<Self> {
        if method.eq_ignore_ascii_case("Delete") {
            Some(Self::Delete)
        } else if method.eq_ignore_ascii_case("Get") {
            Some(Self::Get)
        } else if method.eq_ignore_ascii_case("Head") {
            Some(Self::Head)
        } else if method.eq_ignore_ascii_case("Post") {
            Some(Self::Post)
        } else if method.eq_ignore_ascii_case("Put") {
            Some(Self::Put)
        } else if method.eq_ignore_ascii_case("Connect") {
            Some(Self::Connect)
        } else if method.eq_ignore_ascii_case("Options") {
            Some(Self::Options)
        } else if method.eq_ignore_ascii_case("Trace") {
            Some(Self::Trace)
        } else if method.eq_ignore_ascii_case("Copy") {
            Some(Self::Copy)
        } else if method.eq_ignore_ascii_case("Lock") {
            Some(Self::Lock)
        } else if method.eq_ignore_ascii_case("MkCol") {
            Some(Self::MkCol)
        } else if method.eq_ignore_ascii_case("Move") {
            Some(Self::Move)
        } else if method.eq_ignore_ascii_case("Propfind") {
            Some(Self::Propfind)
        } else if method.eq_ignore_ascii_case("Proppatch") {
            Some(Self::Proppatch)
        } else if method.eq_ignore_ascii_case("Search") {
            Some(Self::Search)
        } else if method.eq_ignore_ascii_case("Unlock") {
            Some(Self::Unlock)
        } else if method.eq_ignore_ascii_case("Bind") {
            Some(Self::Bind)
        } else if method.eq_ignore_ascii_case("Rebind") {
            Some(Self::Rebind)
        } else if method.eq_ignore_ascii_case("Unbind") {
            Some(Self::Unbind)
        } else if method.eq_ignore_ascii_case("Acl") {
            Some(Self::Acl)
        } else if method.eq_ignore_ascii_case("Report") {
            Some(Self::Report)
        } else if method.eq_ignore_ascii_case("MkActivity") {
            Some(Self::MkActivity)
        } else if method.eq_ignore_ascii_case("Checkout") {
            Some(Self::Checkout)
        } else if method.eq_ignore_ascii_case("Merge") {
            Some(Self::Merge)
        } else if method.eq_ignore_ascii_case("MSearch") {
            Some(Self::MSearch)
        } else if method.eq_ignore_ascii_case("Notify") {
            Some(Self::Notify)
        } else if method.eq_ignore_ascii_case("Subscribe") {
            Some(Self::Subscribe)
        } else if method.eq_ignore_ascii_case("Unsubscribe") {
            Some(Self::Unsubscribe)
        } else if method.eq_ignore_ascii_case("Patch") {
            Some(Self::Patch)
        } else if method.eq_ignore_ascii_case("Purge") {
            Some(Self::Purge)
        } else if method.eq_ignore_ascii_case("MkCalendar") {
            Some(Self::MkCalendar)
        } else if method.eq_ignore_ascii_case("Link") {
            Some(Self::Link)
        } else if method.eq_ignore_ascii_case("Unlink") {
            Some(Self::Unlink)
        } else {
            None
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Connect => "CONNECT",
            Self::Options => "OPTIONS",
            Self::Trace => "TRACE",
            Self::Copy => "COPY",
            Self::Lock => "LOCK",
            Self::MkCol => "MKCOL",
            Self::Move => "MOVE",
            Self::Propfind => "PROPFIND",
            Self::Proppatch => "PROPPATCH",
            Self::Search => "SEARCH",
            Self::Unlock => "UNLOCK",
            Self::Bind => "BIND",
            Self::Rebind => "REBIND",
            Self::Unbind => "UNBIND",
            Self::Acl => "ACL",
            Self::Report => "REPORT",
            Self::MkActivity => "MKACTIVITY",
            Self::Checkout => "CHECKOUT",
            Self::Merge => "MERGE",
            Self::MSearch => "MSEARCH",
            Self::Notify => "NOTIFY",
            Self::Subscribe => "SUBSCRIBE",
            Self::Unsubscribe => "UNSUBSCRIBE",
            Self::Patch => "PATCH",
            Self::Purge => "PURGE",
            Self::MkCalendar => "MKCALENDAR",
            Self::Link => "LINK",
            Self::Unlink => "UNLINK",
        }
    }
}

impl Display for Method {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Method {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "{}", self.as_str())
    }
}

/// HTTP headers
#[derive(Debug)]
pub struct Headers<'b, const N: usize = 64>([httparse::Header<'b>; N]);

impl<'b, const N: usize> Headers<'b, N> {
    /// Create a new Headers instance
    #[inline(always)]
    pub const fn new() -> Self {
        Self([httparse::EMPTY_HEADER; N])
    }

    /// Utility method to return the value of the `Content-Length` header, if present
    pub fn content_len(&self) -> Option<u64> {
        self.get("Content-Length").map(|content_len_str| {
            unwrap!(
                content_len_str.parse::<u64>(),
                "Invalid Content-Length header"
            )
        })
    }

    /// Utility method to return the value of the `Content-Type` header, if present
    pub fn content_type(&self) -> Option<&str> {
        self.get("Content-Type")
    }

    /// Utility method to return the value of the `Content-Encoding` header, if present
    pub fn content_encoding(&self) -> Option<&str> {
        self.get("Content-Encoding")
    }

    /// Utility method to return the value of the `Transfer-Encoding` header, if present
    pub fn transfer_encoding(&self) -> Option<&str> {
        self.get("Transfer-Encoding")
    }

    /// Utility method to return the value of the `Host` header, if present
    pub fn host(&self) -> Option<&str> {
        self.get("Host")
    }

    /// Utility method to return the value of the `Connection` header, if present
    pub fn connection(&self) -> Option<&str> {
        self.get("Connection")
    }

    /// Utility method to return the value of the `Cache-Control` header, if present
    pub fn cache_control(&self) -> Option<&str> {
        self.get("Cache-Control")
    }

    /// Utility method to return the value of the `Upgrade` header, if present
    pub fn upgrade(&self) -> Option<&str> {
        self.get("Upgrade")
    }

    /// Iterate over all headers which have valid UTF-8 values
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.iter_raw()
            .filter_map(|(name, value)| str::from_utf8(value).ok().map(|value| (name, value)))
    }

    /// Iterate over all headers, returning the values as raw byte slices
    pub fn iter_raw(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.0
            .iter()
            .filter(|header| !header.name.is_empty())
            .map(|header| (header.name, header.value))
    }

    /// Get the value of a header by name
    pub fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find(|(hname, _)| name.eq_ignore_ascii_case(hname))
            .map(|(_, value)| value)
    }

    /// Get the raw value of a header by name, returning the value as a raw byte slice
    pub fn get_raw(&self, name: &str) -> Option<&[u8]> {
        self.iter_raw()
            .find(|(hname, _)| name.eq_ignore_ascii_case(hname))
            .map(|(_, value)| value)
    }

    /// Set a header by name and value
    pub fn set(&mut self, name: &'b str, value: &'b str) -> &mut Self {
        self.set_raw(name, value.as_bytes())
    }

    /// Set a header by name and value, using a raw byte slice for the value
    pub fn set_raw(&mut self, name: &'b str, value: &'b [u8]) -> &mut Self {
        if !name.is_empty() {
            for header in &mut self.0 {
                if header.name.is_empty() || header.name.eq_ignore_ascii_case(name) {
                    *header = Header { name, value };
                    return self;
                }
            }

            panic!("No space left");
        } else {
            self.remove(name)
        }
    }

    /// Remove a header by name
    pub fn remove(&mut self, name: &str) -> &mut Self {
        let index = self
            .0
            .iter()
            .enumerate()
            .find(|(_, header)| header.name.eq_ignore_ascii_case(name));

        if let Some((mut index, _)) = index {
            while index < self.0.len() - 1 {
                self.0[index] = self.0[index + 1];

                index += 1;
            }

            self.0[index] = EMPTY_HEADER;
        }

        self
    }

    /// A utility method to set the `Content-Length` header
    pub fn set_content_len(
        &mut self,
        content_len: u64,
        buf: &'b mut heapless::String<20>,
    ) -> &mut Self {
        *buf = unwrap!(content_len.try_into());

        self.set("Content-Length", buf.as_str())
    }

    /// A utility method to set the `Content-Type` header
    pub fn set_content_type(&mut self, content_type: &'b str) -> &mut Self {
        self.set("Content-Type", content_type)
    }

    /// A utility method to set the `Content-Encoding` header
    pub fn set_content_encoding(&mut self, content_encoding: &'b str) -> &mut Self {
        self.set("Content-Encoding", content_encoding)
    }

    /// A utility method to set the `Transfer-Encoding` header
    pub fn set_transfer_encoding(&mut self, transfer_encoding: &'b str) -> &mut Self {
        self.set("Transfer-Encoding", transfer_encoding)
    }

    /// A utility method to set the `Transfer-Encoding: Chunked` header
    pub fn set_transfer_encoding_chunked(&mut self) -> &mut Self {
        self.set_transfer_encoding("Chunked")
    }

    /// A utility method to set the `Host` header
    pub fn set_host(&mut self, host: &'b str) -> &mut Self {
        self.set("Host", host)
    }

    /// A utility method to set the `Connection` header
    pub fn set_connection(&mut self, connection: &'b str) -> &mut Self {
        self.set("Connection", connection)
    }

    /// A utility method to set the `Connection: Close` header
    pub fn set_connection_close(&mut self) -> &mut Self {
        self.set_connection("Close")
    }

    /// A utility method to set the `Connection: Keep-Alive` header
    pub fn set_connection_keep_alive(&mut self) -> &mut Self {
        self.set_connection("Keep-Alive")
    }

    /// A utility method to set the `Connection: Upgrade` header
    pub fn set_connection_upgrade(&mut self) -> &mut Self {
        self.set_connection("Upgrade")
    }

    /// A utility method to set the `Cache-Control` header
    pub fn set_cache_control(&mut self, cache: &'b str) -> &mut Self {
        self.set("Cache-Control", cache)
    }

    /// A utility method to set the `Cache-Control: No-Cache` header
    pub fn set_cache_control_no_cache(&mut self) -> &mut Self {
        self.set_cache_control("No-Cache")
    }

    /// A utility method to set the `Upgrade` header
    pub fn set_upgrade(&mut self, upgrade: &'b str) -> &mut Self {
        self.set("Upgrade", upgrade)
    }

    /// A utility method to set the `Upgrade: websocket` header
    pub fn set_upgrade_websocket(&mut self) -> &mut Self {
        self.set_upgrade("websocket")
    }

    /// A utility method to set all Websocket upgrade request headers,
    /// including the `Sec-WebSocket-Key` header with the base64-encoded nonce
    pub fn set_ws_upgrade_request_headers(
        &mut self,
        host: Option<&'b str>,
        origin: Option<&'b str>,
        version: Option<&'b str>,
        nonce: &[u8; ws::NONCE_LEN],
        buf: &'b mut [u8; ws::MAX_BASE64_KEY_LEN],
    ) -> &mut Self {
        for (name, value) in ws::upgrade_request_headers(host, origin, version, nonce, buf) {
            self.set(name, value);
        }

        self
    }

    /// A utility method to set all Websocket upgrade response headers
    /// including the `Sec-WebSocket-Accept` header with the base64-encoded response
    pub fn set_ws_upgrade_response_headers<'a, H>(
        &mut self,
        request_headers: H,
        version: Option<&'a str>,
        buf: &'b mut [u8; ws::MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> Result<&mut Self, ws::UpgradeError>
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        for (name, value) in ws::upgrade_response_headers(request_headers, version, buf)? {
            self.set(name, value);
        }

        Ok(self)
    }
}

impl<const N: usize> Default for Headers<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection type
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ConnectionType {
    KeepAlive,
    Close,
    Upgrade,
}

impl ConnectionType {
    /// Resolve the connection type
    ///
    /// Resolution is based on:
    /// - The connection type found in the headers, if any
    /// - (if the above is missing) based on the carry-over connection type, if any
    /// - (if the above is missing) based on the HTTP version
    ///
    /// Parameters:
    /// - `headers_connection_type`: The connection type found in the headers, if any
    /// - `carry_over_connection_type`: The carry-over connection type
    ///   (i.e. if this is a response, the `carry_over_connection_type` is the connection type of the request)
    /// - `http11`: Whether the HTTP protocol is 1.1
    pub fn resolve(
        headers_connection_type: Option<ConnectionType>,
        carry_over_connection_type: Option<ConnectionType>,
        http11: bool,
    ) -> Result<Self, HeadersMismatchError> {
        match headers_connection_type {
            Some(connection_type) => {
                if let Some(carry_over_connection_type) = carry_over_connection_type {
                    if matches!(connection_type, ConnectionType::KeepAlive)
                        && matches!(carry_over_connection_type, ConnectionType::Close)
                    {
                        warn!("Cannot set a Keep-Alive connection when the peer requested Close");
                        Err(HeadersMismatchError::ResponseConnectionTypeMismatchError)?;
                    }
                }

                Ok(connection_type)
            }
            None => {
                if let Some(carry_over_connection_type) = carry_over_connection_type {
                    Ok(carry_over_connection_type)
                } else if http11 {
                    Ok(Self::KeepAlive)
                } else {
                    Ok(Self::Close)
                }
            }
        }
    }

    /// Create a connection type from a header
    ///
    /// If the header is not a `Connection` header, this method returns `None`
    pub fn from_header(name: &str, value: &str) -> Option<Self> {
        if "Connection".eq_ignore_ascii_case(name) && value.eq_ignore_ascii_case("Close") {
            Some(Self::Close)
        } else if "Connection".eq_ignore_ascii_case(name)
            && value.eq_ignore_ascii_case("Keep-Alive")
        {
            Some(Self::KeepAlive)
        } else if "Connection".eq_ignore_ascii_case(name) && value.eq_ignore_ascii_case("Upgrade") {
            Some(Self::Upgrade)
        } else {
            None
        }
    }

    /// Create a connection type from headers
    ///
    /// If multiple `Connection` headers are found, this method logs a warning and returns the last one
    /// If no `Connection` headers are found, this method returns `None`
    pub fn from_headers<'a, H>(headers: H) -> Option<Self>
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut connection = None;

        for (name, value) in headers {
            let header_connection = Self::from_header(name, value);

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
        }

        connection
    }

    /// Create a raw header from the connection type
    pub fn raw_header(&self) -> (&str, &[u8]) {
        let connection = match self {
            Self::KeepAlive => "Keep-Alive",
            Self::Close => "Close",
            Self::Upgrade => "Upgrade",
        };

        ("Connection", connection.as_bytes())
    }
}

impl Display for ConnectionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::KeepAlive => write!(f, "Keep-Alive"),
            Self::Close => write!(f, "Close"),
            Self::Upgrade => write!(f, "Upgrade"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ConnectionType {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::KeepAlive => defmt::write!(f, "Keep-Alive"),
            Self::Close => defmt::write!(f, "Close"),
            Self::Upgrade => defmt::write!(f, "Upgrade"),
        }
    }
}

/// Body type
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BodyType {
    /// Chunked body (Transfer-Encoding: Chunked)
    Chunked,
    /// Content-length body (Content-Length: {len})
    ContentLen(u64),
    /// Raw body - can only be used with responses, where the connection type is `Close`
    Raw,
}

impl BodyType {
    /// Resolve the body type
    ///
    /// Resolution is based on:
    /// - The body type found in the headers (i.e. `Content-Length` and/or `Transfer-Encoding`), if any
    /// - (if the above is missing) based on the resolved connection type, HTTP protocol and whether we are dealing with a request or a response
    ///
    /// Parameters:
    /// - `headers_body_type`: The body type found in the headers, if any
    /// - `connection_type`: The resolved connection type
    /// - `request`: Whether we are dealing with a request or a response
    /// - `http11`: Whether the HTTP protocol is 1.1
    /// - `chunked_if_unspecified`: (HTTP1.1 only) Upgrades the body type to Chunked if requested so and if no body was specified in the headers
    pub fn resolve(
        headers_body_type: Option<BodyType>,
        connection_type: ConnectionType,
        request: bool,
        http11: bool,
        chunked_if_unspecified: bool,
    ) -> Result<Self, HeadersMismatchError> {
        match headers_body_type {
            Some(headers_body_type) => {
                match headers_body_type {
                    BodyType::Raw => {
                        if request {
                            warn!("Raw body in a request. This is not allowed.");
                            Err(HeadersMismatchError::BodyTypeError(
                                "Raw body in a request. This is not allowed.",
                            ))?;
                        } else if !matches!(connection_type, ConnectionType::Close) {
                            warn!("Raw body response with a Keep-Alive connection. This is not allowed.");
                            Err(HeadersMismatchError::BodyTypeError("Raw body response with a Keep-Alive connection. This is not allowed."))?;
                        }
                    }
                    BodyType::Chunked => {
                        if !http11 {
                            warn!("Chunked body with an HTTP/1.0 connection. This is not allowed.");
                            Err(HeadersMismatchError::BodyTypeError(
                                "Chunked body with an HTTP/1.0 connection. This is not allowed.",
                            ))?;
                        }
                    }
                    _ => {}
                }

                Ok(headers_body_type)
            }
            None => {
                if request {
                    if chunked_if_unspecified && http11 {
                        // With HTTP1.1 we can safely upgrade the body to a chunked one
                        Ok(BodyType::Chunked)
                    } else {
                        debug!("Unknown body type in a request. Assuming Content-Length=0.");
                        Ok(BodyType::ContentLen(0))
                    }
                } else if matches!(connection_type, ConnectionType::Close) {
                    Ok(BodyType::Raw)
                } else if matches!(connection_type, ConnectionType::Upgrade) {
                    if http11 {
                        debug!("Unknown body type in response but the Connection is Upgrade. Assuming Content-Length=0.");
                        Ok(BodyType::ContentLen(0))
                    } else {
                        warn!("Connection is set to Upgrade but the HTTP protocol version is not 1.1. This is not allowed.");
                        Err(HeadersMismatchError::BodyTypeError(
                            "Connection is set to Upgrade but the HTTP protocol version is not 1.1. This is not allowed.",
                        ))
                    }
                } else if chunked_if_unspecified && http11 {
                    // With HTTP1.1 we can safely upgrade the body to a chunked one
                    Ok(BodyType::Chunked)
                } else {
                    warn!("Unknown body type in a response with a Keep-Alive connection. This is not allowed.");
                    Err(HeadersMismatchError::BodyTypeError("Unknown body type in a response with a Keep-Alive connection. This is not allowed."))
                }
            }
        }
    }

    /// Create a body type from a header
    ///
    /// If the header is not a `Content-Length` or `Transfer-Encoding` header, this method returns `None`
    pub fn from_header(name: &str, value: &str) -> Option<Self> {
        if "Transfer-Encoding".eq_ignore_ascii_case(name) {
            if value.eq_ignore_ascii_case("Chunked") {
                return Some(Self::Chunked);
            }
        } else if "Content-Length".eq_ignore_ascii_case(name) {
            return Some(Self::ContentLen(unwrap!(
                value.parse::<u64>(),
                "Invalid Content-Length header"
            ))); // TODO
        }

        None
    }

    /// Create a body type from headers
    ///
    /// If multiple body type headers are found, this method logs a warning and returns the last one
    /// If no body type headers are found, this method returns `None`
    pub fn from_headers<'a, H>(headers: H) -> Option<Self>
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut body = None;

        for (name, value) in headers {
            let header_body = Self::from_header(name, value);

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
        }

        body
    }

    /// Create a raw header from the body type
    ///
    /// If the body type is `Raw`, this method returns `None` as a raw body cannot be
    /// represented in a header and is rather, a consequence of using connection type `Close`
    /// with HTTP server responses
    pub fn raw_header<'a>(&self, buf: &'a mut heapless::String<20>) -> Option<(&str, &'a [u8])> {
        match self {
            Self::Chunked => Some(("Transfer-Encoding", "Chunked".as_bytes())),
            Self::ContentLen(len) => {
                use core::fmt::Write;

                buf.clear();

                write_unwrap!(buf, "{}", len);

                Some(("Content-Length", buf.as_bytes()))
            }
            Self::Raw => None,
        }
    }
}

impl Display for BodyType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Chunked => write!(f, "Chunked"),
            Self::ContentLen(len) => write!(f, "Content-Length: {len}"),
            Self::Raw => write!(f, "Raw"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for BodyType {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Chunked => defmt::write!(f, "Chunked"),
            Self::ContentLen(len) => defmt::write!(f, "Content-Length: {}", len),
            Self::Raw => defmt::write!(f, "Raw"),
        }
    }
}

/// Request headers including the request line (method, path)
#[derive(Debug)]
pub struct RequestHeaders<'b, const N: usize> {
    /// Whether the request is HTTP/1.1
    pub http11: bool,
    /// The HTTP method
    pub method: Method,
    /// The request path
    pub path: &'b str,
    /// The headers
    pub headers: Headers<'b, N>,
}

impl<const N: usize> RequestHeaders<'_, N> {
    // Create a new RequestHeaders instance, defaults to GET / HTTP/1.1
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            http11: true,
            method: Method::Get,
            path: "/",
            headers: Headers::new(),
        }
    }

    /// A utility method to check if the request is a Websocket upgrade request
    pub fn is_ws_upgrade_request(&self) -> bool {
        is_upgrade_request(self.method, self.headers.iter())
    }
}

impl<const N: usize> Default for RequestHeaders<'_, N> {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Display for RequestHeaders<'_, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ", if self.http11 { "HTTP/1.1" } else { "HTTP/1.0" })?;

        writeln!(f, "{} {}", self.method, self.path)?;

        for (name, value) in self.headers.iter() {
            if name.is_empty() {
                break;
            }

            writeln!(f, "{name}: {value}")?;
        }

        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl<const N: usize> defmt::Format for RequestHeaders<'_, N> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "{} ", if self.http11 { "HTTP/1.1" } else { "HTTP/1.0" });

        defmt::write!(f, "{} {}\n", self.method, self.path);

        for (name, value) in self.headers.iter() {
            if name.is_empty() {
                break;
            }

            defmt::write!(f, "{}: {}\n", name, value);
        }
    }
}

/// Response headers including the response line (HTTP version, status code, reason phrase)
#[derive(Debug)]
pub struct ResponseHeaders<'b, const N: usize> {
    /// Whether the response is HTTP/1.1
    pub http11: bool,
    /// The status code
    pub code: u16,
    /// The reason phrase, if present
    pub reason: Option<&'b str>,
    /// The headers
    pub headers: Headers<'b, N>,
}

impl<const N: usize> ResponseHeaders<'_, N> {
    /// Create a new ResponseHeaders instance, defaults to HTTP/1.1 200 OK
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            http11: true,
            code: 200,
            reason: None,
            headers: Headers::new(),
        }
    }

    /// A utility method to check if the response is a Websocket upgrade response
    /// and if the upgrade was accepted
    pub fn is_ws_upgrade_accepted(
        &self,
        nonce: &[u8; NONCE_LEN],
        buf: &mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> bool {
        is_upgrade_accepted(self.code, self.headers.iter(), nonce, buf)
    }
}

impl<const N: usize> Default for ResponseHeaders<'_, N> {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Display for ResponseHeaders<'_, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ", if self.http11 { "HTTP/1.1 " } else { "HTTP/1.0" })?;

        writeln!(f, "{} {}", self.code, self.reason.unwrap_or(""))?;

        for (name, value) in self.headers.iter() {
            if name.is_empty() {
                break;
            }

            writeln!(f, "{name}: {value}")?;
        }

        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl<const N: usize> defmt::Format for ResponseHeaders<'_, N> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "{} ", if self.http11 { "HTTP/1.1 " } else { "HTTP/1.0" });

        defmt::write!(f, "{} {}\n", self.code, self.reason.unwrap_or(""));

        for (name, value) in self.headers.iter() {
            if name.is_empty() {
                break;
            }

            defmt::write!(f, "{}: {}\n", name, value);
        }
    }
}

/// Websocket utilities
pub mod ws {
    use base64::Engine;

    use crate::Method;

    pub const NONCE_LEN: usize = 16;
    pub const MAX_BASE64_KEY_LEN: usize = 28;
    pub const MAX_BASE64_KEY_RESPONSE_LEN: usize = 33;

    pub const UPGRADE_REQUEST_HEADERS_LEN: usize = 7;
    pub const UPGRADE_RESPONSE_HEADERS_LEN: usize = 4;

    /// Return ready-to-use WS upgrade request headers
    ///
    /// Parameters:
    /// - `host`: The `Host` header, if present
    /// - `origin`: The `Origin` header, if present
    /// - `version`: The `Sec-WebSocket-Version` header, if present; otherwise version "13" is assumed
    /// - `nonce`: The nonce to use for the `Sec-WebSocket-Key` header
    /// - `buf`: A buffer to use for base64 encoding the nonce
    pub fn upgrade_request_headers<'a>(
        host: Option<&'a str>,
        origin: Option<&'a str>,
        version: Option<&'a str>,
        nonce: &[u8; NONCE_LEN],
        buf: &'a mut [u8; MAX_BASE64_KEY_LEN],
    ) -> [(&'a str, &'a str); UPGRADE_REQUEST_HEADERS_LEN] {
        let host = host.map(|host| ("Host", host)).unwrap_or(("", ""));
        let origin = origin.map(|origin| ("Origin", origin)).unwrap_or(("", ""));

        [
            host,
            origin,
            ("Content-Length", "0"),
            ("Connection", "Upgrade"),
            ("Upgrade", "websocket"),
            ("Sec-WebSocket-Version", version.unwrap_or("13")),
            ("Sec-WebSocket-Key", sec_key_encode(nonce, buf)),
        ]
    }

    /// Check if the request is a Websocket upgrade request
    pub fn is_upgrade_request<'a, H>(method: Method, request_headers: H) -> bool
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        if method != Method::Get {
            return false;
        }

        let mut connection = false;
        let mut upgrade = false;

        for (name, value) in request_headers {
            if name.eq_ignore_ascii_case("Connection") {
                connection = value.eq_ignore_ascii_case("Upgrade");
            } else if name.eq_ignore_ascii_case("Upgrade") {
                upgrade = value.eq_ignore_ascii_case("websocket");
            }
        }

        connection && upgrade
    }

    /// Websocket upgrade errors
    #[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
    pub enum UpgradeError {
        /// No `Sec-WebSocket-Version` header
        NoVersion,
        /// No `Sec-WebSocket-Key` header
        NoSecKey,
        /// Unsupported `Sec-WebSocket-Version`
        UnsupportedVersion,
    }

    impl core::fmt::Display for UpgradeError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::NoVersion => write!(f, "No Sec-WebSocket-Version header"),
                Self::NoSecKey => write!(f, "No Sec-WebSocket-Key header"),
                Self::UnsupportedVersion => write!(f, "Unsupported Sec-WebSocket-Version"),
            }
        }
    }

    #[cfg(feature = "defmt")]
    impl defmt::Format for UpgradeError {
        fn format(&self, f: defmt::Formatter<'_>) {
            match self {
                Self::NoVersion => defmt::write!(f, "No Sec-WebSocket-Version header"),
                Self::NoSecKey => defmt::write!(f, "No Sec-WebSocket-Key header"),
                Self::UnsupportedVersion => defmt::write!(f, "Unsupported Sec-WebSocket-Version"),
            }
        }
    }

    impl core::error::Error for UpgradeError {}

    /// Return ready-to-use WS upgrade response headers
    ///
    /// Parameters:
    /// - `request_headers`: The request headers
    /// - `version`: The `Sec-WebSocket-Version` header, if present; otherwise version "13" is assumed
    /// - `buf`: A buffer to use for base64 encoding bits and pieces of the response
    pub fn upgrade_response_headers<'a, 'b, H>(
        request_headers: H,
        version: Option<&'a str>,
        buf: &'b mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> Result<[(&'b str, &'b str); UPGRADE_RESPONSE_HEADERS_LEN], UpgradeError>
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut version_ok = false;
        let mut sec_key_resp_len = None;

        for (name, value) in request_headers {
            if name.eq_ignore_ascii_case("Sec-WebSocket-Version") {
                if !value.eq_ignore_ascii_case(version.unwrap_or("13")) {
                    return Err(UpgradeError::NoVersion);
                }

                version_ok = true;
            } else if name.eq_ignore_ascii_case("Sec-WebSocket-Key") {
                sec_key_resp_len = Some(sec_key_response(value, buf).len());
            }
        }

        if version_ok {
            if let Some(sec_key_resp_len) = sec_key_resp_len {
                Ok([
                    ("Content-Length", "0"),
                    ("Connection", "Upgrade"),
                    ("Upgrade", "websocket"),
                    (
                        "Sec-WebSocket-Accept",
                        unwrap!(core::str::from_utf8(&buf[..sec_key_resp_len]).map_err(|_| ())),
                    ),
                ])
            } else {
                Err(UpgradeError::NoSecKey)
            }
        } else {
            Err(UpgradeError::NoVersion)
        }
    }

    /// Check if the response is a Websocket upgrade response and if the upgrade was accepted
    ///
    /// Parameters:
    /// - `code`: The status response code
    /// - `response_headers`: The response headers
    /// - `nonce`: The nonce used for the `Sec-WebSocket-Key` header in the WS upgrade request
    /// - `buf`: A buffer to use when performing the check
    pub fn is_upgrade_accepted<'a, H>(
        code: u16,
        response_headers: H,
        nonce: &[u8; NONCE_LEN],
        buf: &'a mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> bool
    where
        H: IntoIterator<Item = (&'a str, &'a str)>,
    {
        if code != 101 {
            return false;
        }

        let mut connection = false;
        let mut upgrade = false;
        let mut sec_key_response = false;

        for (name, value) in response_headers {
            if name.eq_ignore_ascii_case("Connection") {
                connection = value.eq_ignore_ascii_case("Upgrade");
            } else if name.eq_ignore_ascii_case("Upgrade") {
                upgrade = value.eq_ignore_ascii_case("websocket");
            } else if name.eq_ignore_ascii_case("Sec-WebSocket-Accept") {
                let sec_key = sec_key_encode(nonce, buf);

                let mut sha1 = sha1_smol::Sha1::new();
                sha1.update(sec_key.as_bytes());

                let sec_key_resp = sec_key_response_finalize(&mut sha1, buf);

                sec_key_response = value.eq(sec_key_resp);
            }
        }

        connection && upgrade && sec_key_response
    }

    fn sec_key_encode<'a>(nonce: &[u8], buf: &'a mut [u8]) -> &'a str {
        let nonce_base64_len = unwrap!(base64::engine::general_purpose::STANDARD
            .encode_slice(nonce, buf)
            .map_err(|_| ()));

        unwrap!(core::str::from_utf8(&buf[..nonce_base64_len]).map_err(|_| ()))
    }

    /// Compute the response for a given `Sec-WebSocket-Key`
    pub fn sec_key_response<'a>(
        sec_key: &str,
        buf: &'a mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> &'a str {
        let mut sha1 = sha1_smol::Sha1::new();

        sec_key_response_start(sec_key, &mut sha1);
        sec_key_response_finalize(&mut sha1, buf)
    }

    fn sec_key_response_start(sec_key: &str, sha1: &mut sha1_smol::Sha1) {
        debug!("Computing response for key: {}", sec_key);

        sha1.update(sec_key.as_bytes());
    }

    fn sec_key_response_finalize<'a>(sha1: &mut sha1_smol::Sha1, buf: &'a mut [u8]) -> &'a str {
        const WS_MAGIC_GUUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

        sha1.update(WS_MAGIC_GUUID.as_bytes());

        let len = unwrap!(base64::engine::general_purpose::STANDARD
            .encode_slice(sha1.digest().bytes(), buf)
            .map_err(|_| ()));

        let sec_key_response = unwrap!(core::str::from_utf8(&buf[..len]).map_err(|_| ()));

        debug!("Computed response: {}", sec_key_response);

        sec_key_response
    }
}

#[cfg(test)]
mod test {
    use crate::{
        ws::{sec_key_response, MAX_BASE64_KEY_RESPONSE_LEN},
        BodyType, ConnectionType,
    };

    #[test]
    fn test_resp() {
        let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
        let resp = sec_key_response("dGhlIHNhbXBsZSBub25jZQ==", &mut buf);

        assert_eq!(resp, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn test_resolve_conn() {
        // Default connection type resolution
        assert_eq!(
            unwrap!(ConnectionType::resolve(None, None, true)),
            ConnectionType::KeepAlive
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(None, None, false)),
            ConnectionType::Close
        );

        // Connection type resolution based on carry-over (for responses)
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                None,
                Some(ConnectionType::KeepAlive),
                false
            )),
            ConnectionType::KeepAlive
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                None,
                Some(ConnectionType::KeepAlive),
                true
            )),
            ConnectionType::KeepAlive
        );

        // Connection type resoluton based on the header value
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                None,
                false
            )),
            ConnectionType::Close
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::KeepAlive),
                None,
                false
            )),
            ConnectionType::KeepAlive
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                None,
                true
            )),
            ConnectionType::Close
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::KeepAlive),
                None,
                true
            )),
            ConnectionType::KeepAlive
        );

        // Connection type in the headers should aggree with the carry-over one
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                Some(ConnectionType::Close),
                false
            )),
            ConnectionType::Close
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::KeepAlive),
                Some(ConnectionType::KeepAlive),
                false
            )),
            ConnectionType::KeepAlive
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                Some(ConnectionType::Close),
                true
            )),
            ConnectionType::Close
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::KeepAlive),
                Some(ConnectionType::KeepAlive),
                true
            )),
            ConnectionType::KeepAlive
        );
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                Some(ConnectionType::KeepAlive),
                false
            )),
            ConnectionType::Close
        );
        assert!(ConnectionType::resolve(
            Some(ConnectionType::KeepAlive),
            Some(ConnectionType::Close),
            false
        )
        .is_err());
        assert_eq!(
            unwrap!(ConnectionType::resolve(
                Some(ConnectionType::Close),
                Some(ConnectionType::KeepAlive),
                true
            )),
            ConnectionType::Close
        );
        assert!(ConnectionType::resolve(
            Some(ConnectionType::KeepAlive),
            Some(ConnectionType::Close),
            true
        )
        .is_err());
    }

    #[test]
    fn test_resolve_body() {
        // Request with no body type specified means Content-Length=0
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::KeepAlive,
                true,
                true,
                false
            )),
            BodyType::ContentLen(0)
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Close,
                true,
                true,
                false
            )),
            BodyType::ContentLen(0)
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::KeepAlive,
                true,
                false,
                false
            )),
            BodyType::ContentLen(0)
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Close,
                true,
                false,
                false
            )),
            BodyType::ContentLen(0)
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Upgrade,
                false,
                true,
                false
            )),
            BodyType::ContentLen(0)
        );

        // Receiving a response with no body type after requesting a connection upgrade with
        // HTTP1.0 is no allowed.
        assert!(BodyType::resolve(None, ConnectionType::Upgrade, false, false, false).is_err());

        // Request or response with a chunked body type is invalid for HTTP1.0
        assert!(BodyType::resolve(
            Some(BodyType::Chunked),
            ConnectionType::Close,
            true,
            false,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Chunked),
            ConnectionType::KeepAlive,
            true,
            false,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Chunked),
            ConnectionType::Close,
            false,
            false,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Chunked),
            ConnectionType::KeepAlive,
            false,
            false,
            false
        )
        .is_err());

        // Raw body in a request is not allowed
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::Close,
            true,
            true,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::KeepAlive,
            true,
            true,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::Close,
            true,
            false,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::KeepAlive,
            true,
            false,
            false
        )
        .is_err());

        // Raw body in a response with a Keep-Alive connection is not allowed
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::KeepAlive,
            false,
            true,
            false
        )
        .is_err());
        assert!(BodyType::resolve(
            Some(BodyType::Raw),
            ConnectionType::KeepAlive,
            false,
            false,
            false
        )
        .is_err());

        // The same, but with a Close connection IS allowed
        assert_eq!(
            unwrap!(BodyType::resolve(
                Some(BodyType::Raw),
                ConnectionType::Close,
                false,
                true,
                false
            )),
            BodyType::Raw
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                Some(BodyType::Raw),
                ConnectionType::Close,
                false,
                false,
                false
            )),
            BodyType::Raw
        );

        // Request upgrades to chunked encoding should only work for HTTP1.1, and if there is no body type in the headers
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Close,
                true,
                true,
                true
            )),
            BodyType::Chunked
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::KeepAlive,
                true,
                true,
                true
            )),
            BodyType::Chunked
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Close,
                true,
                false,
                true
            )),
            BodyType::ContentLen(0)
        );
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::KeepAlive,
                true,
                false,
                true
            )),
            BodyType::ContentLen(0)
        );

        // Response upgrades to chunked encoding should only work for HTTP1.1, and if there is no body type in the headers, and if the connection is KeepAlive
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::KeepAlive,
                false,
                true,
                true
            )),
            BodyType::Chunked
        );
        // Response upgrades should not be honored if the connection is Close
        assert_eq!(
            unwrap!(BodyType::resolve(
                None,
                ConnectionType::Close,
                false,
                true,
                true
            )),
            BodyType::Raw
        );
    }
}
