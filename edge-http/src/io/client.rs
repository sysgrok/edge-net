use core::mem;
use core::net::SocketAddr;
use core::str;

use embedded_io_async::{ErrorType, Read, Write};

use edge_nal::{Close, TcpConnect, TcpShutdown};

use crate::{
    ws::{upgrade_request_headers, MAX_BASE64_KEY_LEN, MAX_BASE64_KEY_RESPONSE_LEN, NONCE_LEN},
    ConnectionType, DEFAULT_MAX_HEADERS_COUNT,
};

use super::{
    is_interim, send_headers, send_request, Body, Error, Framing, ResponseHeaders, SendBody,
};

use super::Method;

const COMPLETION_BUF_SIZE: usize = 64;

/// A client connection that can be used to send HTTP requests and receive responses.
#[allow(private_interfaces)]
pub enum Connection<'b, T, const N: usize = DEFAULT_MAX_HEADERS_COUNT>
where
    T: TcpConnect,
{
    Transition(TransitionState),
    Unbound(UnboundState<'b, T, N>),
    Request(RequestState<'b, T, N>),
    Response(ResponseState<'b, T, N>),
}

impl<'b, T, const N: usize> Connection<'b, T, N>
where
    T: TcpConnect,
{
    /// Create a new client connection.
    ///
    /// Note that the connection does not have any built-in read/write timeouts:
    /// - To add a timeout on each IO operation, wrap the `socket` type with the `edge_nal::WithTimeout` wrapper.
    /// - To add a global request-response timeout, wrap your complete request-response processing
    ///   logic with the `edge_nal::with_timeout` function.
    ///
    /// Parameters:
    /// - `buf`: A buffer to use for reading and writing data.
    /// - `socket`: The TCP stack to use for the connection.
    /// - `addr`: The address of the server to connect to.
    pub fn new(buf: &'b mut [u8], socket: &'b T, addr: SocketAddr) -> Self {
        Self::Unbound(UnboundState {
            buf,
            socket,
            addr,
            io: None,
        })
    }

    /// Reinitialize the connection with a new address.
    pub async fn reinitialize(&mut self, addr: SocketAddr) -> Result<(), Error<T::Error>> {
        let _ = self.complete().await;
        unwrap!(self.unbound_mut(), "Unreachable").addr = addr;

        Ok(())
    }

    /// Initiate an HTTP request.
    pub async fn initiate_request(
        &mut self,
        http11: bool,
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> Result<(), Error<T::Error>> {
        self.start_request(http11, method, uri, headers).await
    }

    /// A utility method to initiate a WebSocket upgrade request.
    pub async fn initiate_ws_upgrade_request(
        &mut self,
        host: Option<&str>,
        origin: Option<&str>,
        uri: &str,
        version: Option<&str>,
        nonce: &[u8; NONCE_LEN],
        nonce_base64_buf: &mut [u8; MAX_BASE64_KEY_LEN],
    ) -> Result<(), Error<T::Error>> {
        let headers = upgrade_request_headers(host, origin, version, nonce, nonce_base64_buf);

        self.initiate_request(true, Method::Get, uri, &headers)
            .await
    }

    /// Return `true` if a request has been initiated.
    pub fn is_request_initiated(&self) -> bool {
        matches!(self, Self::Request(_))
    }

    /// Initiate an HTTP response.
    ///
    /// This should be called after a request has been initiated and the request body had been sent.
    pub async fn initiate_response(&mut self) -> Result<(), Error<T::Error>> {
        self.complete_request().await
    }

    /// Return `true` if a response has been initiated.
    pub fn is_response_initiated(&self) -> bool {
        matches!(self, Self::Response(_))
    }

    /// Return `true` if the server accepted the WebSocket upgrade request.
    pub fn is_ws_upgrade_accepted(
        &self,
        nonce: &[u8; NONCE_LEN],
        buf: &mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> Result<bool, Error<T::Error>> {
        Ok(self.headers()?.is_ws_upgrade_accepted(nonce, buf))
    }

    /// Split the connection into its headers and body parts.
    ///
    /// The connection must be in response mode.
    #[allow(clippy::type_complexity)]
    pub fn split(&mut self) -> (&ResponseHeaders<'b, N>, &mut Body<'b, T::Socket<'b>>) {
        let response = self.response_mut().expect("Not in response mode");

        (&response.response, &mut response.io)
    }

    /// Get the headers of the response.
    ///
    /// The connection must be in response mode.
    pub fn headers(&self) -> Result<&ResponseHeaders<'b, N>, Error<T::Error>> {
        let response = self.response_ref()?;

        Ok(&response.response)
    }

    /// Get a mutable reference to the raw connection.
    ///
    /// This can be used to send raw data over the connection.
    pub fn raw_connection(&mut self) -> Result<&mut T::Socket<'b>, Error<T::Error>> {
        Ok(self.io_mut())
    }

    /// Release the connection, returning the raw connection and the buffer.
    pub fn release(mut self) -> (T::Socket<'b>, &'b mut [u8]) {
        let mut state = self.unbind();

        let io = unwrap!(state.io.take());

        (io, state.buf)
    }

    async fn start_request(
        &mut self,
        http11: bool,
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> Result<(), Error<T::Error>> {
        let _ = self.complete().await;

        let state = self.unbound_mut()?;

        let fresh_connection = if state.io.is_none() {
            state.io = Some(state.socket.connect(state.addr).await.map_err(Error::Io)?);
            true
        } else {
            false
        };

        let mut state = self.unbind();

        let result = async {
            match send_request(http11, method, uri, unwrap!(state.io.as_mut())).await {
                Ok(_) => (),
                Err(Error::Io(_)) => {
                    if !fresh_connection {
                        // Attempt to reconnect and re-send the request
                        state.io = None;
                        state.io = Some(state.socket.connect(state.addr).await.map_err(Error::Io)?);

                        send_request(http11, method, uri, unwrap!(state.io.as_mut())).await?;
                    }
                }
                Err(other) => Err(other)?,
            }

            let io = unwrap!(state.io.as_mut());

            send_headers(
                headers,
                None,
                true,
                http11,
                Framing::ChunkedIfUnspecified,
                &mut *io,
            )
            .await
        }
        .await;

        match result {
            Ok((connection_type, body_type)) => {
                *self = Self::Request(RequestState {
                    buf: state.buf,
                    socket: state.socket,
                    addr: state.addr,
                    method,
                    connection_type,
                    io: SendBody::new(body_type, unwrap!(state.io)),
                });

                Ok(())
            }
            Err(e) => {
                state.io = None;
                *self = Self::Unbound(state);

                Err(e)
            }
        }
    }

    /// Complete the request-response cycle
    ///
    /// If the request has not been initiated, this method will do nothing.
    /// If the response has not been initiated, it will be initiated and will be consumed.
    pub async fn complete(&mut self) -> Result<(), Error<T::Error>> {
        let result = async {
            if self.request_mut().is_ok() {
                self.complete_request().await?;
            }

            let needs_close = if self.response_mut().is_ok() {
                self.complete_response().await?
            } else {
                false
            };

            Result::<_, Error<T::Error>>::Ok(needs_close)
        }
        .await;

        let mut state = self.unbind();

        match result {
            Ok(true) | Err(_) => {
                let io = state.io.take();
                *self = Self::Unbound(state);

                if let Some(mut io) = io {
                    io.close(Close::Both).await.map_err(Error::Io)?;
                    let _ = io.abort().await;
                }
            }
            _ => {
                *self = Self::Unbound(state);
            }
        };

        result?;

        Ok(())
    }

    pub async fn close(mut self) -> Result<(), Error<T::Error>> {
        let res = self.complete().await;

        if let Some(mut io) = self.unbind().io.take() {
            io.close(Close::Both).await.map_err(Error::Io)?;
            let _ = io.abort().await;
        }

        res
    }

    async fn complete_request(&mut self) -> Result<(), Error<T::Error>> {
        self.request_mut()?.io.finish().await?;

        let request_connection_type = self.request_mut()?.connection_type;
        let method = self.request_mut()?.method;

        let mut state = self.unbind();
        let buf_ptr: *mut [u8] = state.buf;

        let result = loop {
            // Fresh headers each time, so none from an interim response remain
            let mut response = ResponseHeaders::new();

            // The headers are read exactly, so the next response is not in `buf` yet
            let received = response
                .receive(
                    unwrap!(unsafe { buf_ptr.as_mut() }),
                    &mut unwrap!(state.io.as_mut()),
                    true,
                )
                .await;

            match received {
                Ok(_) if is_interim(response.code) => {
                    debug!("Skipping interim response {}", response.code);
                }
                Ok((buf, read_len)) => {
                    break response
                        .resolve_for_request(method, request_connection_type)
                        .map(|types| (response, buf, read_len, types));
                }
                Err(e) => break Err(e),
            }
        };

        match result {
            Ok((response, buf, read_len, (connection_type, body_type))) => {
                let io = Body::new(body_type, buf, read_len, unwrap!(state.io));

                *self = Self::Response(ResponseState {
                    buf: buf_ptr,
                    response,
                    socket: state.socket,
                    addr: state.addr,
                    connection_type,
                    io,
                });

                Ok(())
            }
            Err(e) => {
                state.io = None;
                state.buf = unwrap!(unsafe { buf_ptr.as_mut() });

                *self = Self::Unbound(state);

                Err(e)
            }
        }
    }

    async fn complete_response(&mut self) -> Result<bool, Error<T::Error>> {
        if self.request_mut().is_ok() {
            self.complete_request().await?;
        }

        let response = self.response_mut()?;

        let mut buf = [0; COMPLETION_BUF_SIZE];
        while response.io.read(&mut buf).await? > 0 {}

        let needs_close = response.needs_close();

        *self = Self::Unbound(self.unbind());

        Ok(needs_close)
    }

    /// Return `true` if the connection needs to be closed (i.e. the server has requested it or the connection is in an invalid state)
    pub fn needs_close(&self) -> bool {
        match self {
            Self::Response(response) => response.needs_close(),
            _ => true,
        }
    }

    fn unbind(&mut self) -> UnboundState<'b, T, N> {
        let state = mem::replace(self, Self::Transition(TransitionState(())));

        match state {
            Self::Unbound(unbound) => unbound,
            Self::Request(request) => {
                let io = request.io.release();

                UnboundState {
                    buf: request.buf,
                    socket: request.socket,
                    addr: request.addr,
                    io: Some(io),
                }
            }
            Self::Response(response) => {
                let io = response.io.release();

                UnboundState {
                    buf: unwrap!(unsafe { response.buf.as_mut() }),
                    socket: response.socket,
                    addr: response.addr,
                    io: Some(io),
                }
            }
            _ => unreachable!(),
        }
    }

    fn unbound_mut(&mut self) -> Result<&mut UnboundState<'b, T, N>, Error<T::Error>> {
        if let Self::Unbound(new) = self {
            Ok(new)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn request_mut(&mut self) -> Result<&mut RequestState<'b, T, N>, Error<T::Error>> {
        if let Self::Request(request) = self {
            Ok(request)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn response_mut(&mut self) -> Result<&mut ResponseState<'b, T, N>, Error<T::Error>> {
        if let Self::Response(response) = self {
            Ok(response)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn response_ref(&self) -> Result<&ResponseState<'b, T, N>, Error<T::Error>> {
        if let Self::Response(response) = self {
            Ok(response)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn io_mut(&mut self) -> &mut T::Socket<'b> {
        match self {
            Self::Unbound(unbound) => unwrap!(unbound.io.as_mut()),
            Self::Request(request) => request.io.as_raw_writer(),
            Self::Response(response) => response.io.as_raw_reader(),
            _ => unreachable!(),
        }
    }
}

impl<T, const N: usize> ErrorType for Connection<'_, T, N>
where
    T: TcpConnect,
{
    type Error = Error<T::Error>;
}

impl<T, const N: usize> Read for Connection<'_, T, N>
where
    T: TcpConnect,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.response_mut()?.io.read(buf).await
    }
}

impl<T, const N: usize> Write for Connection<'_, T, N>
where
    T: TcpConnect,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.request_mut()?.io.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.request_mut()?.io.flush().await
    }
}

struct TransitionState(());

struct UnboundState<'b, T, const N: usize>
where
    T: TcpConnect,
{
    buf: &'b mut [u8],
    socket: &'b T,
    addr: SocketAddr,
    io: Option<T::Socket<'b>>,
}

struct RequestState<'b, T, const N: usize>
where
    T: TcpConnect,
{
    buf: &'b mut [u8],
    socket: &'b T,
    addr: SocketAddr,
    method: Method,
    connection_type: ConnectionType,
    io: SendBody<T::Socket<'b>>,
}

struct ResponseState<'b, T, const N: usize>
where
    T: TcpConnect,
{
    buf: *mut [u8],
    response: ResponseHeaders<'b, N>,
    socket: &'b T,
    addr: SocketAddr,
    connection_type: ConnectionType,
    io: Body<'b, T::Socket<'b>>,
}

impl<T, const N: usize> ResponseState<'_, T, N>
where
    T: TcpConnect,
{
    fn needs_close(&self) -> bool {
        matches!(self.connection_type, ConnectionType::Close) || self.io.needs_close()
    }
}

#[cfg(test)]
#[allow(clippy::large_futures)]
mod test {
    use core::cell::RefCell;
    use core::convert::Infallible;
    use core::net::SocketAddr;

    use embassy_futures::block_on;
    use embedded_io_async::{ErrorType, Read, Write};

    use edge_nal::{Close, Readable, TcpConnect, TcpShutdown, TcpSplit};

    use crate::io::Error;
    use crate::ws::{MAX_BASE64_KEY_LEN, MAX_BASE64_KEY_RESPONSE_LEN};
    use crate::Method;

    use super::Connection;

    struct Inner<'a> {
        input: &'a [u8],
        connects: usize,
    }

    /// A network whose sockets all read from one fixed input; writes are discarded
    struct MockNet<'a>(RefCell<Inner<'a>>);

    impl<'a> MockNet<'a> {
        fn new(input: &'a [u8]) -> Self {
            Self(RefCell::new(Inner { input, connects: 0 }))
        }
    }

    #[derive(Clone, Copy)]
    struct MockSocket<'n, 'a>(&'n RefCell<Inner<'a>>);

    impl<'a> TcpConnect for MockNet<'a> {
        type Error = Infallible;

        type Socket<'n>
            = MockSocket<'n, 'a>
        where
            Self: 'n;

        async fn connect(&self, _remote: SocketAddr) -> Result<Self::Socket<'_>, Self::Error> {
            self.0.borrow_mut().connects += 1;

            Ok(MockSocket(&self.0))
        }
    }

    impl ErrorType for MockSocket<'_, '_> {
        type Error = Infallible;
    }

    impl Read for MockSocket<'_, '_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let mut inner = self.0.borrow_mut();
            let len = core::cmp::min(buf.len(), inner.input.len());
            buf[..len].copy_from_slice(&inner.input[..len]);
            inner.input = &inner.input[len..];

            Ok(len)
        }
    }

    impl Write for MockSocket<'_, '_> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl Readable for MockSocket<'_, '_> {
        async fn readable(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl<'n, 'a> TcpSplit for MockSocket<'n, 'a> {
        type Read<'s>
            = MockSocket<'n, 'a>
        where
            Self: 's;
        type Write<'s>
            = MockSocket<'n, 'a>
        where
            Self: 's;

        fn split(&mut self) -> (Self::Read<'_>, Self::Write<'_>) {
            (*self, *self)
        }
    }

    impl TcpShutdown for MockSocket<'_, '_> {
        async fn close(&mut self, _what: Close) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn abort(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    const ADDR: SocketAddr =
        SocketAddr::new(core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST), 80);

    /// Send a `method` request on `connection` and read the whole response body into `body`
    async fn request(
        connection: &mut Connection<'_, MockNet<'_>, 8>,
        method: Method,
        body: &mut [u8],
    ) -> Result<(u16, usize), Error<Infallible>> {
        connection.initiate_request(true, method, "/", &[]).await?;
        connection.initiate_response().await?;

        let code = connection.headers()?.code;
        let (_, response_body) = connection.split();
        let mut len = 0;
        loop {
            let n = response_body.read(&mut body[len..]).await?;
            if n == 0 {
                break;
            }
            len += n;
        }

        connection.complete().await?;

        Ok((code, len))
    }

    #[test]
    fn test_responses_without_body() {
        block_on(async {
            let net = MockNet::new(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n\
                  HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n\
                  HTTP/1.1 304 Not Modified\r\nContent-Length: 5\r\n\r\n\
                  HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            );

            let mut buf = [0; 512];
            let mut connection = Connection::<_, 8>::new(&mut buf, &net, ADDR);
            let mut body = [0; 16];

            assert_eq!(
                request(&mut connection, Method::Head, &mut body).await,
                Ok((200, 0))
            );
            assert_eq!(
                request(&mut connection, Method::Get, &mut body).await,
                Ok((204, 0))
            );
            assert_eq!(
                request(&mut connection, Method::Get, &mut body).await,
                Ok((304, 0))
            );

            assert_eq!(
                request(&mut connection, Method::Get, &mut body).await,
                Ok((200, 5))
            );
            assert_eq!(&body[..5], b"hello");

            assert_eq!(net.0.borrow().connects, 1);
        })
    }

    #[test]
    fn test_interim_responses() {
        block_on(async {
            let net = MockNet::new(
                b"HTTP/1.1 100 Continue\r\n\r\n\
                  HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n\
                  HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            );

            let mut buf = [0; 512];
            let mut connection = Connection::<_, 8>::new(&mut buf, &net, ADDR);
            let mut body = [0; 16];

            connection
                .initiate_request(true, Method::Get, "/", &[])
                .await
                .unwrap();
            connection.initiate_response().await.unwrap();

            // Only the headers of the final response are seen
            let headers = connection.headers().unwrap();
            assert_eq!(headers.code, 200);
            assert_eq!(headers.headers.get("Link"), None);
            assert_eq!(headers.headers.content_len(), Ok(Some(5)));

            let (_, response_body) = connection.split();
            assert_eq!(response_body.read(&mut body).await, Ok(5));
            assert_eq!(&body[..5], b"hello");
            connection.complete().await.unwrap();
        })
    }

    #[test]
    fn test_interim_response_only() {
        block_on(async {
            let net = MockNet::new(b"HTTP/1.1 100 Continue\r\n\r\n");

            let mut buf = [0; 512];
            let mut connection = Connection::<_, 8>::new(&mut buf, &net, ADDR);

            connection
                .initiate_request(true, Method::Get, "/", &[])
                .await
                .unwrap();
            assert_eq!(
                connection.initiate_response().await,
                Err(Error::ConnectionClosed)
            );

            // The connection is left unbound and can still be completed
            assert!(!connection.is_request_initiated());
            assert!(!connection.is_response_initiated());
            connection.complete().await.unwrap();
        })
    }

    #[test]
    fn test_ws_upgrade_response() {
        block_on(async {
            // RFC 6455's sample nonce and the matching accept key
            let nonce = *b"the sample nonce";
            let net = MockNet::new(
                b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\
                  Upgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\nws",
            );

            let mut buf = [0; 512];
            let mut connection = Connection::<_, 8>::new(&mut buf, &net, ADDR);
            let mut key_buf = [0; MAX_BASE64_KEY_LEN];
            connection
                .initiate_ws_upgrade_request(None, None, "/ws", None, &nonce, &mut key_buf)
                .await
                .unwrap();
            connection.initiate_response().await.unwrap();

            let mut accept_buf = [0; MAX_BASE64_KEY_RESPONSE_LEN];
            assert!(connection
                .is_ws_upgrade_accepted(&nonce, &mut accept_buf)
                .unwrap());

            // The bytes after the 101 are left for the new protocol
            let mut data = [0; 8];
            let (_, body) = connection.split();
            assert_eq!(body.read(&mut data).await, Ok(0));
            let socket = connection.raw_connection().unwrap();
            assert_eq!(socket.read(&mut data).await, Ok(2));
            assert_eq!(&data[..2], b"ws");
        })
    }
}
