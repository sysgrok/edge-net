use core::fmt::{self, Debug, Display};
use core::mem::{self, MaybeUninit};
use core::pin::pin;

use edge_nal::{
    with_timeout, Close, Readable, TcpShutdown, TcpSplit, WithTimeout, WithTimeoutError,
};

use embedded_io_async::{ErrorType, Read, Write};

use super::{send_headers, send_status, Body, Error, RequestHeaders, SendBody};

use crate::ws::{upgrade_response_headers, MAX_BASE64_KEY_RESPONSE_LEN};
use crate::{ConnectionType, DEFAULT_MAX_HEADERS_COUNT};

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

impl<C, E> embedded_io_async::Error for HandleRequestError<C, E>
where
    C: Debug + core::error::Error + embedded_io_async::Error,
    E: Debug + core::error::Error,
{
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            Self::Connection(Error::Io(e)) => e.kind(),
            _ => embedded_io_async::ErrorKind::Other,
        }
    }
}

impl<C, E> core::error::Error for HandleRequestError<C, E>
where
    C: core::error::Error,
    E: core::error::Error,
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
    /// A note on concurrent connection acceptance:
    /// - All handler tasks concurrently call `accept()` to maximize connection acceptance capacity.
    /// - This is critical for TCP stacks without accept queues (e.g., smoltcp/embassy-net), where
    ///   incoming connections can only be accepted if at least one task is actively waiting in `accept()`.
    /// - When a task is busy handling a request, other tasks remain available to accept new connections.
    /// - **Threading safety**: The acceptor must be safe to use from multiple async tasks.
    ///   For single-threaded async executors (e.g., embassy-executor on embedded platforms),
    ///   acceptors using non-Sync types like `Cell` for internal state are safe.
    ///   For multi-threaded executors, the acceptor's internal state must be properly synchronized
    ///   (e.g., using atomics or locks).
    ///
    /// Consider using `run_with_socket_queue()` instead for better connection handling
    /// with TCP stacks that lack accept queues (e.g., smoltcp/embassy-net). The socket queue architecture
    /// decouples connection acceptance from HTTP processing, allowing connections to be accepted even when
    /// all worker tasks are busy.
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
        let mut tasks = heapless::Vec::<_, P>::new();

        info!(
            "Creating {} handler tasks, memory: {}B",
            P,
            core::mem::size_of_val(&tasks)
        );

        for index in 0..P {
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

                        let io = acceptor.accept().await.map_err(Error::Io)?.1;

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

    /// Run the server with a socket queue architecture (recommended for smoltcp/embassy-net)
    ///
    /// This method addresses the limitation of TCP stacks without accept queues (e.g., smoltcp/embassy-net)
    /// by using a signal-based coordination mechanism between acceptor and worker tasks. This ensures that
    /// the number of sockets in use never exceeds the available socket pool size, preventing resource exhaustion.
    ///
    /// # Type Parameters
    ///
    /// - `Q`: Number of acceptor tasks and maximum sockets that can be allocated simultaneously (default: 8)
    ///
    /// # Important Constraints
    ///
    /// **CRITICAL**: The `Q` parameter must satisfy these constraints or the function will panic:
    /// - `Q` must be **less than or equal to** the number of sockets in your smoltcp/embassy-net socket pool
    ///   - If `Q` exceeds the socket pool size, accept() calls will fail and cause panics
    /// - `Q` should be **greater than or equal to** `P` for the architecture to provide benefits
    ///   - If `Q < P`, you lose the advantage of decoupling acceptance from processing
    ///   - Recommended: `Q >= P` (e.g., Q=8 with P=4)
    ///
    /// # Timeout Configuration
    ///
    /// The function does NOT establish timeouts by default (except optional keepalive timeout):
    /// - Wrap the acceptor with `edge_nal::WithTimeout` to set socket-level timeouts
    /// - Wrap handler logic with `edge_nal::with_timeout` for request-response timeouts
    ///
    /// # Parameters
    ///
    /// - `keepalive_timeout_ms`: Optional timeout in milliseconds for idle keepalive connections
    /// - `acceptor`: An implementation of `edge_nal::TcpAccept` to accept incoming connections
    /// - `handler`: An implementation of `Handler` to handle incoming requests
    #[cfg(feature = "io")]
    #[inline(never)]
    #[cold]
    pub async fn run_with_socket_queue<A, H, const Q: usize>(
        &mut self,
        keepalive_timeout_ms: Option<u32>,
        acceptor: A,
        handler: H,
    ) -> Result<(), Error<A::Error>>
    where
        A: edge_nal::TcpAccept,
        H: Handler,
    {
        use embassy_sync::blocking_mutex::raw::NoopRawMutex;
        use embassy_sync::channel::Channel;
        use embassy_sync::signal::Signal;

        // ============================================================================
        // Internal Architecture
        // ============================================================================
        // This implementation uses a signal-based coordination mechanism:
        //
        // - Q acceptor tasks: Each waits for a signal before calling accept()
        // - Each acceptor has its own signal indicating socket availability
        // - Initially all Q signals are set, allowing Q concurrent accepts
        // - When an acceptor accepts a connection, it enqueues (socket, acceptor_id)
        // - P worker tasks dequeue from channel and process HTTP requests
        // - When a worker finishes, it signals the specific acceptor that provided the socket
        // - At most Q sockets allocated at any time (some accepting, some processing)
        //
        // This prevents socket pool exhaustion that would occur if all Q acceptors
        // were simultaneously calling accept() while P workers were processing,
        // which would require Q+P sockets. By tracking which acceptor provided each
        // socket, workers signal the correct acceptor (one waiting on its signal,
        // not busy on accept()).
        //
        // Threading: Uses NoopRawMutex for single-threaded executors (embassy-executor).
        // For multi-threaded executors, a different mutex type would be needed.
        // ============================================================================

        // Create a channel to pass accepted sockets from acceptor tasks to worker tasks
        // Each message contains the socket and the ID of the acceptor that accepted it
        let socket_queue = Channel::<NoopRawMutex, (A::Socket<'_>, usize), Q>::new();

        // Create signals for each acceptor task to coordinate socket availability
        // When a worker finishes processing a socket, it signals an acceptor to accept a new connection
        // This ensures we never have more sockets in use than available in the pool
        let accept_signals: [Signal<NoopRawMutex, ()>; Q] = [(); Q].map(|_| Signal::new());

        // Create Q acceptor tasks - each waits for its signal before accepting
        // This ensures we never have more than (Q - sockets_in_use) acceptors calling accept()
        let mut acceptor_tasks = heapless::Vec::<_, Q>::new();

        info!(
            "Creating {} acceptor tasks and {} worker tasks, queue size: {}",
            Q, P, Q
        );

        for acceptor_id in 0..Q {
            let acceptor = &acceptor;
            let socket_queue = &socket_queue;
            let signal = &accept_signals[acceptor_id];

            unwrap!(acceptor_tasks
                .push(async move {
                    loop {
                        // Wait for signal that a socket is available
                        // Initially all signals are ready, allowing Q concurrent accepts
                        signal.wait().await;

                        debug!(
                            "Acceptor task {}: Got signal, waiting for connection",
                            display2format!(acceptor_id)
                        );

                        match acceptor.accept().await {
                            Ok((_, io)) => {
                                debug!(
                                    "Acceptor task {}: Got connection, enqueueing",
                                    display2format!(acceptor_id)
                                );

                                // Send the socket along with the acceptor ID to the queue
                                // This allows workers to signal the correct acceptor when done
                                socket_queue.send((io, acceptor_id)).await;

                                debug!(
                                    "Acceptor task {}: Connection enqueued",
                                    display2format!(acceptor_id)
                                );
                            }
                            Err(e) => {
                                warn!(
                                    "Acceptor task {}: Error accepting connection: {:?}",
                                    display2format!(acceptor_id),
                                    debug2format!(e)
                                );
                                // Signal ourselves again to retry
                                signal.signal(());
                            }
                        }
                    }
                })
                .map_err(|_| ()));
        }

        // Initially signal all acceptors to start accepting
        for signal in &accept_signals {
            signal.signal(());
        }

        // Create worker tasks
        let mut worker_tasks = heapless::Vec::<_, P>::new();

        for index in 0..P {
            let task_id = index;
            let handler = &handler;
            let socket_queue = &socket_queue;
            let accept_signals = &accept_signals;
            // Safety: The server buffer array is properly initialized (MaybeUninit is used correctly),
            // and each worker task gets exclusive access to its own buffer slice via its unique index.
            // The pointer remains valid for the lifetime of the server and the buffer is not moved.
            let buf: *mut [u8; B] = &mut unsafe { self.0.assume_init_mut() }[index];

            unwrap!(worker_tasks
                .push(async move {
                    loop {
                        debug!(
                            "Worker task {}: Waiting for connection from queue",
                            display2format!(task_id)
                        );

                        // Receive an accepted socket from the queue along with the acceptor ID
                        let (io, acceptor_id) = socket_queue.receive().await;

                        debug!(
                            "Worker task {}: Got connection from acceptor {} from queue",
                            display2format!(task_id),
                            display2format!(acceptor_id)
                        );

                        handle_connection::<_, _, N>(
                            io,
                            unwrap!(unsafe { buf.as_mut() }),
                            keepalive_timeout_ms,
                            task_id,
                            handler,
                        )
                        .await;

                        // Signal the specific acceptor that provided this socket
                        // This ensures we signal an acceptor that's waiting on its signal,
                        // not one that might be busy waiting on accept()
                        debug!(
                            "Worker task {}: Finished processing, signaling acceptor {}",
                            display2format!(task_id),
                            display2format!(acceptor_id)
                        );
                        accept_signals[acceptor_id].signal(());
                    }
                })
                .map_err(|_| ()));
        }

        // Pin tasks for select_slice
        let acceptor_tasks = pin!(acceptor_tasks);
        let acceptor_tasks = unsafe { acceptor_tasks.map_unchecked_mut(|t| t.as_mut_slice()) };

        let worker_tasks = pin!(worker_tasks);
        let worker_tasks = unsafe { worker_tasks.map_unchecked_mut(|t| t.as_mut_slice()) };

        // Run all acceptor and worker tasks concurrently
        // Use select to run both acceptors and workers, return if any completes
        use embassy_futures::select::Either;
        let result = embassy_futures::select::select(
            async {
                let (result, _acceptor_index): (Result<(), Error<A::Error>>, _) =
                    embassy_futures::select::select_slice(acceptor_tasks).await;
                result
            },
            async {
                let (result, _worker_index): (Result<(), Error<A::Error>>, _) =
                    embassy_futures::select::select_slice(worker_tasks).await;
                result
            },
        )
        .await;

        // Neither acceptor nor worker tasks should complete normally
        match result {
            Either::First(Err(e)) => {
                warn!("Acceptor task quit with error: {:?}", debug2format!(e));
                Err(e)
            }
            Either::First(Ok(_)) => {
                warn!("Acceptor task quit unexpectedly");
                Ok(())
            }
            Either::Second(Err(e)) => {
                warn!("Worker task quit with error: {:?}", debug2format!(e));
                Err(e)
            }
            Either::Second(Ok(_)) => {
                warn!("Worker task quit unexpectedly");
                Ok(())
            }
        }
    }
}

impl<const P: usize, const B: usize, const N: usize> Default for Server<P, B, N> {
    fn default() -> Self {
        Self::new()
    }
}
