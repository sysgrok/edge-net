use core::mem;
use core::net::SocketAddr;
use core::str;

use embedded_io_async::{ErrorType, Read, Write};

use edge_nal::{Close, TcpConnect, TcpShutdown};

use crate::{
    ws::{upgrade_request_headers, MAX_BASE64_KEY_LEN, MAX_BASE64_KEY_RESPONSE_LEN, NONCE_LEN},
    ConnectionType, DEFAULT_MAX_HEADERS_COUNT,
};

use super::{send_headers, send_request, Body, Error, ResponseHeaders, SendBody};

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

            send_headers(headers, None, true, http11, true, &mut *io).await
        }
        .await;

        match result {
            Ok((connection_type, body_type)) => {
                *self = Self::Request(RequestState {
                    buf: state.buf,
                    socket: state.socket,
                    addr: state.addr,
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

        let mut state = self.unbind();
        let buf_ptr: *mut [u8] = state.buf;
        let mut response = ResponseHeaders::new();

        match response
            .receive(state.buf, &mut unwrap!(state.io.as_mut()), true)
            .await
        {
            Ok((buf, read_len)) => {
                let (connection_type, body_type) =
                    response.resolve::<T::Error>(request_connection_type)?;

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
