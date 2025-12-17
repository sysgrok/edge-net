use core::fmt::{self, Debug, Display};
use core::mem::{self, MaybeUninit};
use core::pin::pin;

use edge_nal::{
    with_timeout, Close, Readable, TcpShutdown, TcpSplit, WithTimeout, WithTimeoutError,
};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;

use embedded_io_async::{ErrorType, Read, Write};

use super::{send_headers, send_status, Body, Error, RequestHeaders, SendBody};

use crate::ws::{upgrade_response_headers, MAX_BASE64_KEY_RESPONSE_LEN};
use crate::{ConnectionType, DEFAULT_MAX_HEADERS_COUNT};

#[allow(unused_imports)]
#[cfg(feature = "embedded-svc")]
pub use embedded_svc_compat::*;

pub const DEFAULT_HANDLER_TASKS_COUNT: usize = 4;
pub const DEFAULT_BUF_SIZE: usize = 2048;

const COMPLETION_BUF_SIZE: usize = 64;

/// A connection state machine for handling HTTP server requests-response cycles.
#[allow(private_interfaces)]
pub enum Connection<'b, T, const N: usize = DEFAULT_MAX_HEADERS_COUNT> {
    Transition(TransitionState),
    Unbound(T),
    Request(RequestState<'b, T, N>),
    Response(ResponseState<T>),
}

impl<'b, T, const N: usize> Connection<'b, T, N>
where
    T: Read + Write,
{
    /// Create a new connection state machine for an incoming request
    ///
    /// Note that the connection does not have any built-in read/write timeouts:
    /// - To add a timeout on each IO operation, wrap the `io` type with the `edge_nal::WithTimeout` wrapper.
    /// - To add a global request-response timeout, wrap your complete request-response processing
    ///   logic with the `edge_nal::with_timeout` function.
    ///
    /// Parameters:
    /// - `buf`: A buffer to store the request headers
    /// - `io`: A socket stream
    pub async fn new(
        buf: &'b mut [u8],
        mut io: T,
    ) -> Result<Connection<'b, T, N>, Error<T::Error>> {
        let mut request = RequestHeaders::new();

        let (buf, read_len) = request.receive(buf, &mut io, true).await?;

        let (connection_type, body_type) = request.resolve::<T::Error>()?;

        let io = Body::new(body_type, buf, read_len, io);

        Ok(Self::Request(RequestState {
            request,
            io,
            connection_type,
        }))
    }

    /// Return `true` of the connection is in request state (i.e. the initial state upon calling `new`)
    pub fn is_request_initiated(&self) -> bool {
        matches!(self, Self::Request(_))
    }

    /// Split the connection into request headers and body
    pub fn split(&mut self) -> (&RequestHeaders<'b, N>, &mut Body<'b, T>) {
        let req = self.request_mut().expect("Not in request mode");

        (&req.request, &mut req.io)
    }

    /// Return a reference to the request headers
    pub fn headers(&self) -> Result<&RequestHeaders<'b, N>, Error<T::Error>> {
        Ok(&self.request_ref()?.request)
    }

    /// Return `true` if the request is a WebSocket upgrade request
    pub fn is_ws_upgrade_request(&self) -> Result<bool, Error<T::Error>> {
        Ok(self.headers()?.is_ws_upgrade_request())
    }

    /// Switch the connection into a response state
    ///
    /// Parameters:
    /// - `status`: The HTTP status code
    /// - `message`: An optional HTTP status message
    /// - `headers`: An array of HTTP response headers.
    ///   Note that if no `Content-Length` or `Transfer-Encoding` headers are provided,
    ///   the body will be send with chunked encoding (for HTTP1.1 only and if the connection is not Close)
    pub async fn initiate_response(
        &mut self,
        status: u16,
        message: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<(), Error<T::Error>> {
        self.complete_request(status, message, headers).await
    }

    /// A convenience method to initiate a WebSocket upgrade response
    pub async fn initiate_ws_upgrade_response(
        &mut self,
        buf: &mut [u8; MAX_BASE64_KEY_RESPONSE_LEN],
    ) -> Result<(), Error<T::Error>> {
        let headers = upgrade_response_headers(self.headers()?.headers.iter(), None, buf)?;

        self.initiate_response(101, None, &headers).await
    }

    /// Return `true` if the connection is in response state
    pub fn is_response_initiated(&self) -> bool {
        matches!(self, Self::Response(_))
    }

    /// Completes the response and switches the connection back to the unbound state
    /// If the connection is still in a request state, and empty 200 OK response is sent
    pub async fn complete(&mut self) -> Result<(), Error<T::Error>> {
        if self.is_request_initiated() {
            self.complete_request(200, Some("OK"), &[]).await?;
        }

        if self.is_response_initiated() {
            self.complete_response().await?;
        }

        Ok(())
    }

    /// Completes the response with an error message and switches the connection back to the unbound state
    ///
    /// If the connection is still in a request state, an empty 500 Internal Error response is sent
    pub async fn complete_err(&mut self, err: &str) -> Result<(), Error<T::Error>> {
        let result = self.request_mut();

        match result {
            Ok(_) => {
                let headers = [("Connection", "Close"), ("Content-Type", "text/plain")];

                self.complete_request(500, Some("Internal Error"), &headers)
                    .await?;

                let response = self.response_mut()?;

                response.io.write_all(err.as_bytes()).await?;
                response.io.finish().await?;

                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Return `true` if the connection needs to be closed
    ///
    /// This is determined by the connection type (i.e. `Connection: Close` header)
    pub fn needs_close(&self) -> bool {
        match self {
            Self::Response(response) => response.needs_close(),
            _ => true,
        }
    }

    /// Switch the connection to unbound state, returning a mutable reference to the underlying socket stream
    ///
    /// NOTE: Use with care, and only if the connection is completed in the meantime
    pub fn unbind(&mut self) -> Result<&mut T, Error<T::Error>> {
        let io = self.unbind_mut();
        *self = Self::Unbound(io);

        Ok(self.io_mut())
    }

    async fn complete_request(
        &mut self,
        status: u16,
        reason: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<(), Error<T::Error>> {
        let request = self.request_mut()?;

        let mut buf = [0; COMPLETION_BUF_SIZE];
        while request.io.read(&mut buf).await? > 0 {}

        let http11 = request.request.http11;
        let request_connection_type = request.connection_type;

        let mut io = self.unbind_mut();

        let result = async {
            send_status(http11, status, reason, &mut io).await?;

            let (connection_type, body_type) = send_headers(
                headers.iter(),
                Some(request_connection_type),
                false,
                http11,
                true,
                &mut io,
            )
            .await?;

            Ok((connection_type, body_type))
        }
        .await;

        match result {
            Ok((connection_type, body_type)) => {
                *self = Self::Response(ResponseState {
                    io: SendBody::new(body_type, io),
                    connection_type,
                });

                Ok(())
            }
            Err(e) => {
                *self = Self::Unbound(io);

                Err(e)
            }
        }
    }

    async fn complete_response(&mut self) -> Result<(), Error<T::Error>> {
        self.response_mut()?.io.finish().await?;

        Ok(())
    }

    fn unbind_mut(&mut self) -> T {
        let state = mem::replace(self, Self::Transition(TransitionState(())));

        match state {
            Self::Request(request) => request.io.release(),
            Self::Response(response) => response.io.release(),
            Self::Unbound(io) => io,
            _ => unreachable!(),
        }
    }

    fn request_mut(&mut self) -> Result<&mut RequestState<'b, T, N>, Error<T::Error>> {
        if let Self::Request(request) = self {
            Ok(request)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn request_ref(&self) -> Result<&RequestState<'b, T, N>, Error<T::Error>> {
        if let Self::Request(request) = self {
            Ok(request)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn response_mut(&mut self) -> Result<&mut ResponseState<T>, Error<T::Error>> {
        if let Self::Response(response) = self {
            Ok(response)
        } else {
            Err(Error::InvalidState)
        }
    }

    fn io_mut(&mut self) -> &mut T {
        match self {
            Self::Request(request) => request.io.as_raw_reader(),
            Self::Response(response) => response.io.as_raw_writer(),
            Self::Unbound(io) => io,
            _ => unreachable!(),
        }
    }
}

impl<T, const N: usize> ErrorType for Connection<'_, T, N>
where
    T: ErrorType,
{
    type Error = Error<T::Error>;
}

impl<T, const N: usize> Read for Connection<'_, T, N>
where
    T: Read + Write,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.request_mut()?.io.read(buf).await
    }
}

impl<T, const N: usize> Write for Connection<'_, T, N>
where
    T: Read + Write,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.response_mut()?.io.write(buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.response_mut()?.io.flush().await
    }
}

struct TransitionState(());

struct RequestState<'b, T, const N: usize> {
    request: RequestHeaders<'b, N>,
    io: Body<'b, T>,
    connection_type: ConnectionType,
}

struct ResponseState<T> {
    io: SendBody<T>,
    connection_type: ConnectionType,
}

impl<T> ResponseState<T>
where
    T: Write,
{
    fn needs_close(&self) -> bool {
        matches!(self.connection_type, ConnectionType::Close) || self.io.needs_close()
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HandlerError<T, E> {
    Io(T),
    Connection(Error<T>),
    Handler(E),
}

impl<T, E> From<Error<T>> for HandlerError<T, E> {
    fn from(e: Error<T>) -> Self {
        Self::Connection(e)
    }
}

/// A trait (async callback) for handling incoming HTTP requests
pub trait Handler {
    type Error<E>: Debug
    where
        E: Debug;

    /// Handle an incoming HTTP request
    ///
    /// Parameters:
    /// - `task_id`: An identifier for the task, thast can be used by the handler for logging purposes
    /// - `connection`: A connection state machine for the request-response cycle
    async fn handle<T, const N: usize>(
        &self,
        task_id: impl Display + Copy,
        connection: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit;
}

impl<H> Handler for &H
where
    H: Handler,
{
    type Error<E>
        = H::Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        task_id: impl Display + Copy,
        connection: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        (**self).handle(task_id, connection).await
    }
}

impl<H> Handler for &mut H
where
    H: Handler,
{
    type Error<E>
        = H::Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        task_id: impl Display + Copy,
        connection: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        (**self).handle(task_id, connection).await
    }
}

impl<H> Handler for WithTimeout<H>
where
    H: Handler,
{
    type Error<E>
        = WithTimeoutError<H::Error<E>>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        task_id: impl Display + Copy,
        connection: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        let mut io = pin!(self.io().handle(task_id, connection));

        with_timeout(self.timeout_ms(), &mut io).await?;

        Ok(())
    }
}

/// A convenience function to handle multiple HTTP requests over a single socket stream,
/// using the specified handler.
///
/// The socket stream will be closed only in case of error, or until the client explicitly requests that
/// either with a hard socket close, or with a `Connection: Close` header.
///
/// A note on timeouts:
/// - The function does NOT - by default - establish any timeouts on the IO operations _except_
///   an optional timeout for detecting idle connections, so that they can be closed and thus make
///   the server available for accepting new connections.
///   It is up to the caller to wrap the acceptor type with `edge_nal::WithTimeout` to establish
///   timeouts on the socket produced by the acceptor.
/// - Similarly, the server does NOT establish any timeouts on the complete request-response cycle.
///   It is up to the caller to wrap their complete or partial handling logic with
///   `edge_nal::with_timeout`, or its whole handler with `edge_nal::WithTimeout`, so as to establish
///   a global or semi-global request-response timeout.
///
/// Parameters:
/// - `io`: A socket stream
/// - `buf`: A work-area buffer used by the implementation
/// - `keepalive_timeout_ms`: An optional timeout in milliseconds for detecting an idle keepalive connection
///   that should be closed. If not provided, the server will not close idle connections.
/// - `task_id`: An identifier for the task, used for logging purposes
/// - `handler`: An implementation of `Handler` to handle incoming requests
pub async fn handle_connection<H, T, const N: usize>(
    mut io: T,
    buf: &mut [u8],
    keepalive_timeout_ms: Option<u32>,
    task_id: impl Display + Copy,
    handler: H,
) where
    H: Handler,
    T: Read + Write + Readable + TcpSplit + TcpShutdown,
{
    let close = loop {
        debug!(
            "Handler task {}: Waiting for a new request",
            display2format!(task_id)
        );

        if let Some(keepalive_timeout_ms) = keepalive_timeout_ms {
            let wait_data = with_timeout(keepalive_timeout_ms, io.readable()).await;
            match wait_data {
                Err(WithTimeoutError::Timeout) => {
                    info!(
                        "Handler task {}: Closing connection due to inactivity",
                        display2format!(task_id)
                    );
                    break true;
                }
                Err(e) => {
                    warn!(
                        "Handler task {}: Error when handling request: {:?}",
                        display2format!(task_id),
                        debug2format!(e)
                    );
                    break true;
                }
                Ok(_) => {}
            }
        }

        let result = handle_request::<_, _, N>(buf, &mut io, task_id, &handler).await;

        match result {
            Err(HandlerError::Connection(Error::ConnectionClosed)) => {
                debug!(
                    "Handler task {}: Connection closed",
                    display2format!(task_id)
                );
                break false;
            }
            Err(e) => {
                warn!(
                    "Handler task {}: Error when handling request: {:?}",
                    display2format!(task_id),
                    debug2format!(e)
                );
                break true;
            }
            Ok(needs_close) => {
                if needs_close {
                    debug!(
                        "Handler task {}: Request complete; closing connection",
                        display2format!(task_id)
                    );
                    break true;
                } else {
                    debug!(
                        "Handler task {}: Request complete",
                        display2format!(task_id)
                    );
                }
            }
        }
    };

    if close {
        if let Err(e) = io.close(Close::Both).await {
            warn!(
                "Handler task {}: Error when closing the socket: {:?}",
                display2format!(task_id),
                debug2format!(e)
            );
        }
    } else {
        let _ = io.abort().await;
    }
}

/// The error type for handling HTTP requests
#[derive(Debug)]
pub enum HandleRequestError<C, E> {
    /// A connection error (HTTP protocol error or a socket IO error)
    Connection(Error<C>),
    /// A handler error
    Handler(E),
}

impl<T, E> From<Error<T>> for HandleRequestError<T, E> {
    fn from(e: Error<T>) -> Self {
        Self::Connection(e)
    }
}

impl<C, E> fmt::Display for HandleRequestError<C, E>
where
    C: fmt::Display,
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(e) => write!(f, "Connection error: {}", e),
            Self::Handler(e) => write!(f, "Handler error: {}", e),
        }
    }
}

#[cfg(feature = "defmt")]
impl<C, E> defmt::Format for HandleRequestError<C, E>
where
    C: defmt::Format,
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Connection(e) => defmt::write!(f, "Connection error: {}", e),
            Self::Handler(e) => defmt::write!(f, "Handler error: {}", e),
        }
    }
}

#[cfg(not(feature = "std"))]
impl<C: Debug + embedded_io_async::Error, E: Debug + Display> core::error::Error
    for HandleRequestError<C, E>
{
}

#[cfg(not(feature = "std"))]
impl<C, E> embedded_io_async::Error for HandleRequestError<C, E>
where
    C: Debug + embedded_io_async::Error,
    E: Debug + Display,
{
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            Self::Connection(Error::Io(e)) => e.kind(),
            _ => embedded_io_async::ErrorKind::Other,
        }
    }
}

#[cfg(feature = "std")]
impl<C, E> std::error::Error for HandleRequestError<C, E>
where
    C: std::error::Error,
    E: std::error::Error,
{
}

/// A convenience function to handle a single HTTP request over a socket stream,
/// using the specified handler.
///
/// Note that this function does not set any timeouts on the request-response processing
/// or on the IO operations. It is up that the caller to use the `with_timeout` function
/// and the `WithTimeout` struct from the `edge-nal` crate to wrap the future returned
/// by this function, or the socket stream, or both.
///
/// Parameters:
/// - `buf`: A work-area buffer used by the implementation
/// - `io`: A socket stream
/// - `task_id`: An identifier for the task, used for logging purposes
/// - `handler`: An implementation of `Handler` to handle incoming requests
pub async fn handle_request<H, T, const N: usize>(
    buf: &mut [u8],
    io: T,
    task_id: impl Display + Copy,
    handler: H,
) -> Result<bool, HandlerError<T::Error, H::Error<T::Error>>>
where
    H: Handler,
    T: Read + Write + TcpSplit,
{
    let mut connection = Connection::<_, N>::new(buf, io).await?;

    let result = handler.handle(task_id, &mut connection).await;

    match result {
        Result::Ok(_) => connection.complete().await?,
        Result::Err(e) => connection
            .complete_err("INTERNAL ERROR")
            .await
            .map_err(|_| HandlerError::Handler(e))?,
    }

    Ok(connection.needs_close())
}

/// A type alias for an HTTP server with default buffer sizes.
pub type DefaultServer =
    Server<{ DEFAULT_HANDLER_TASKS_COUNT }, { DEFAULT_BUF_SIZE }, { DEFAULT_MAX_HEADERS_COUNT }>;

/// A type alias for the HTTP server buffers (essentially, arrays of `MaybeUninit`)
pub type ServerBuffers<const P: usize, const B: usize> = MaybeUninit<[[u8; B]; P]>;

/// An HTTP server that can handle multiple requests concurrently.
///
/// The server needs an implementation of `edge_nal::TcpAccept` to accept incoming connections.
#[repr(transparent)]
pub struct Server<
    const P: usize = DEFAULT_HANDLER_TASKS_COUNT,
    const B: usize = DEFAULT_BUF_SIZE,
    const N: usize = DEFAULT_MAX_HEADERS_COUNT,
>(ServerBuffers<P, B>);

impl<const P: usize, const B: usize, const N: usize> Server<P, B, N> {
    /// Create a new HTTP server
    #[inline(always)]
    pub const fn new() -> Self {
        Self(MaybeUninit::uninit())
    }

    /// Run the server with the specified acceptor and handler
    ///
    /// A note on timeouts:
    /// - The function does NOT - by default - establish any timeouts on the IO operations _except_
    ///   an optional timeout on idle connections, so that they can be closed.
    ///   It is up to the caller to wrap the acceptor type with `edge_nal::WithTimeout` to establish
    ///   timeouts on the socket produced by the acceptor.
    /// - Similarly, the function does NOT establish any timeouts on the complete request-response cycle.
    ///   It is up to the caller to wrap their complete or partial handling logic with
    ///   `edge_nal::with_timeout`, or its whole handler with `edge_nal::WithTimeout`, so as to establish
    ///   a global or semi-global request-response timeout.
    ///
    /// Parameters:
    /// - `keepalive_timeout_ms`: An optional timeout in milliseconds for detecting an idle keepalive
    ///   connection that should be closed. If not provided, the function will not close idle connections
    ///   and the connection - in the absence of other timeouts - will remain active forever.
    /// - `acceptor`: An implementation of `edge_nal::TcpAccept` to accept incoming connections
    /// - `handler`: An implementation of `Handler` to handle incoming requests
    ///   If not provided, a default timeout of 50 seconds is used.
    #[inline(never)]
    #[cold]
    pub async fn run<A, H>(
        &mut self,
        keepalive_timeout_ms: Option<u32>,
        acceptor: A,
        handler: H,
    ) -> Result<(), Error<A::Error>>
    where
        A: edge_nal::TcpAccept,
        H: Handler,
    {
        let mutex = Mutex::<NoopRawMutex, _>::new(());
        let mut tasks = heapless::Vec::<_, P>::new();

        info!(
            "Creating {} handler tasks, memory: {}B",
            P,
            core::mem::size_of_val(&tasks)
        );

        for index in 0..P {
            let mutex = &mutex;
            let acceptor = &acceptor;
            let task_id = index;
            let handler = &handler;
            let buf: *mut [u8; B] = &mut unsafe { self.0.assume_init_mut() }[index];

            unwrap!(tasks
                .push(async move {
                    loop {
                        debug!(
                            "Handler task {}: Waiting for connection",
                            display2format!(task_id)
                        );

                        let io = {
                            let _guard = mutex.lock().await;

                            acceptor.accept().await.map_err(Error::Io)?.1
                        };

                        debug!(
                            "Handler task {}: Got connection request",
                            display2format!(task_id)
                        );

                        handle_connection::<_, _, N>(
                            io,
                            unwrap!(unsafe { buf.as_mut() }),
                            keepalive_timeout_ms,
                            task_id,
                            handler,
                        )
                        .await;
                    }
                })
                .map_err(|_| ()));
        }

        let tasks = pin!(tasks);

        let tasks = unsafe { tasks.map_unchecked_mut(|t| t.as_mut_slice()) };
        let (result, _) = embassy_futures::select::select_slice(tasks).await;

        warn!(
            "Server processing loop quit abruptly: {:?}",
            debug2format!(result)
        );

        result
    }
}

impl<const P: usize, const B: usize, const N: usize> Default for Server<P, B, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "embedded-svc")]
mod embedded_svc_compat {
    use embedded_io_async::{Read, Write};

    use embedded_svc::http::server::asynch::{Connection, Headers, Query};

    use crate::io::Body;
    use crate::RequestHeaders;

    impl<T, const N: usize> Headers for super::Connection<'_, T, N>
    where
        T: Read + Write,
    {
        fn header(&self, name: &str) -> Option<&'_ str> {
            self.request_ref()
                .expect("Not in request mode")
                .request
                .header(name)
        }
    }

    impl<T, const N: usize> Query for super::Connection<'_, T, N>
    where
        T: Read + Write,
    {
        fn uri(&self) -> &'_ str {
            self.request_ref()
                .expect("Not in request mode")
                .request
                .uri()
        }

        fn method(&self) -> embedded_svc::http::Method {
            self.request_ref()
                .expect("Not in request mode")
                .request
                .method()
        }
    }

    impl<'b, T, const N: usize> Connection for super::Connection<'b, T, N>
    where
        T: Read + Write,
    {
        type Headers = RequestHeaders<'b, N>;

        type Read = Body<'b, T>;

        type RawConnectionError = T::Error;

        type RawConnection = T;

        fn split(&mut self) -> (&Self::Headers, &mut Self::Read) {
            super::Connection::split(self)
        }

        async fn initiate_response(
            &mut self,
            status: u16,
            message: Option<&str>,
            headers: &[(&str, &str)],
        ) -> Result<(), Self::Error> {
            super::Connection::initiate_response(self, status, message, headers).await
        }

        fn is_response_initiated(&self) -> bool {
            super::Connection::is_response_initiated(self)
        }

        fn raw_connection(&mut self) -> Result<&mut Self::RawConnection, Self::Error> {
            // TODO: Needs a GAT rather than `&mut` return type
            // or `embedded-svc` fully upgraded to async traits & `embedded-io` 0.4 to re-enable
            //ServerConnection::raw_connection(self).map(EmbIo)
            panic!("Not supported")
        }
    }

    // NOTE: Currently, the `edge-http` and the `embedded-svc` Handler traits are
    // incompatible, in that the `edge-http` async `Handler`'s `handle` method is generic,
    // while the `embedded-svc` `Handler`'s `handle` method is not.
    //
    // Code below is commented out until `embedded-svc`'s `Handler` signature is changed
    // to match the `edge-http` `Handler` signature.

    // pub struct SvcHandler<H>(H);

    // impl<'b, T, const N: usize, H> Handler for SvcHandler<H>
    // where
    //     H: embedded_svc::http::server::asynch::Handler<super::Connection<'b, T, N>>,
    //     T: Read + Write,
    // {
    //     type Error<E> = Error<E> where E: Debug;

    //     async fn handle<T, const N: usize>(
    //         &self,
    //         _task_id: impl core::fmt::Display + Copy,
    //         connection: &mut super::Connection<'_, T, N>,
    //     ) -> Result<(), Self::Error<T::Error>>
    //     where
    //         T: Read + Write,
    //     {
    //         unwrap!(self.0.handle(connection).await);

    //         Ok(())
    //     }
    // }
}
