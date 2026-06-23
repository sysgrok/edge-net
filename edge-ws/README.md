# edge-ws

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

Async + `no_std` + no-alloc implementation of the Websockets protocol.

For other protocols, look at the [edge-net](https://github.com/sysgrok/edge-net) aggregator crate documentation.

## Examples

**NOTE**

To connect the Websocket client example to the Websocket server example - rather that to the public Websocket echo server, 
just run it with some argument, i.e.

```sh
./target/debug/examples/ws_client 1
```

### Websocket client

```rust
use core::net::SocketAddr;

use anyhow::bail;

use edge_http::io::client::Connection;
use edge_http::ws::{MAX_BASE64_KEY_LEN, MAX_BASE64_KEY_RESPONSE_LEN, NONCE_LEN};
use edge_nal::{AddrType, Dns, TcpConnect};
use edge_ws::{FrameHeader, FrameType};

use rand::{thread_rng, RngCore};

use log::*;

// NOTE: HTTP-only echo WS servers seem to be hard to find, this one might or might not work...
const PUBLIC_ECHO_SERVER: (&str, u16, &str) = ("websockets.chilkat.io", 80, "/wsChilkatEcho.ashx");
const OUR_ECHO_SERVER: (&str, u16, &str) = ("127.0.0.1", 8881, "/");

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let stack = edge_nal_std::Stack::new();

    let mut buf = [0_u8; 8192];

    futures_lite::future::block_on(work(&stack, &mut buf)).unwrap();
}

async fn work<T: TcpConnect + Dns>(stack: &T, buf: &mut [u8]) -> Result<(), anyhow::Error>
where
    <T as Dns>::Error: Send + Sync + std::error::Error + 'static,
    <T as TcpConnect>::Error: Send + Sync + std::error::Error + 'static,
{
    let mut args = std::env::args();
    args.next(); // Skip the executable name

    let (fqdn, port, path) = if args.next().is_some() {
        OUR_ECHO_SERVER
    } else {
        PUBLIC_ECHO_SERVER
    };

    info!("About to open an HTTP connection to {fqdn} port {port}");

    let ip = stack.get_host_by_name(fqdn, AddrType::IPv4).await?;

    let mut conn: Connection<_> = Connection::new(buf, stack, SocketAddr::new(ip, port));

    let mut rng_source = thread_rng();

    let mut nonce = [0_u8; NONCE_LEN];
    rng_source.fill_bytes(&mut nonce);

    let mut buf = [0_u8; MAX_BASE64_KEY_LEN];
    conn.initiate_ws_upgrade_request(Some(fqdn), Some("foo.com"), path, None, &nonce, &mut buf)
        .await?;
    conn.initiate_response().await?;

    let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
    if !conn.is_ws_upgrade_accepted(&nonce, &mut buf)? {
        bail!("WS upgrade failed");
    }

    conn.complete().await?;

    // Now we have the TCP socket in a state where it can be operated as a WS connection
    // Send some traffic to a WS echo server and read it back

    let (mut socket, buf) = conn.release();

    info!("Connection upgraded to WS, starting traffic now");

    for payload in ["Hello world!", "How are you?", "I'm fine, thanks!"] {
        let header = FrameHeader {
            frame_type: FrameType::Text(false),
            payload_len: payload.len() as _,
            mask_key: rng_source.next_u32().into(),
        };

        info!("Sending {header}, with payload \"{payload}\"");
        header.send(&mut socket).await?;
        header.send_payload(&mut socket, payload.as_bytes()).await?;

        let header = FrameHeader::recv(&mut socket).await?;
        let payload = header.recv_payload(&mut socket, buf).await?;

        match header.frame_type {
            FrameType::Text(_) => {
                info!(
                    "Got {header}, with payload \"{}\"",
                    core::str::from_utf8(payload).unwrap()
                );
            }
            FrameType::Binary(_) => {
                info!("Got {header}, with payload {payload:?}");
            }
            _ => {
                bail!("Unexpected {}", header);
            }
        }

        if !header.frame_type.is_final() {
            bail!("Unexpected fragmented frame");
        }
    }

    // Inform the server we are closing the connection

    let header = FrameHeader {
        frame_type: FrameType::Close,
        payload_len: 0,
        mask_key: rng_source.next_u32().into(),
    };

    info!("Closing");

    header.send(&mut socket).await?;

    Ok(())
}
```

### Websocket echo server

```rust
use core::fmt::{Debug, Display};

use edge_http::io::server::{Connection, DefaultServer, Handler};
use edge_http::io::Error;
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_http::Method;
use edge_nal::TcpBind;
use edge_ws::{FrameHeader, FrameType};

use embedded_io_async::{Read, Write};

use log::info;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let mut server = DefaultServer::new();

    futures_lite::future::block_on(run(&mut server)).unwrap();
}

pub async fn run(server: &mut DefaultServer) -> Result<(), anyhow::Error> {
    let addr = "0.0.0.0:8881";

    info!("Running HTTP server on {addr}");

    let acceptor = edge_nal_std::Stack::new()
        .bind(addr.parse().unwrap())
        .await?;

    server.run(None, acceptor, WsHandler).await?;

    Ok(())
}

#[derive(Debug)]
enum WsHandlerError<C, W> {
    Connection(C),
    Ws(W),
}

impl<C, W> From<C> for WsHandlerError<C, W> {
    fn from(e: C) -> Self {
        Self::Connection(e)
    }
}

struct WsHandler;

impl Handler for WsHandler {
    type Error<E>
        = WsHandlerError<Error<E>, edge_ws::Error<E>>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Clone,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        let headers = conn.headers()?;

        if headers.method != Method::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/" {
            conn.initiate_response(404, Some("Not Found"), &[]).await?;
        } else if !conn.is_ws_upgrade_request()? {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;

            conn.write_all(b"Initiate WS Upgrade request to switch this connection to WS")
                .await?;
        } else {
            let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
            conn.initiate_ws_upgrade_response(&mut buf).await?;

            conn.complete().await?;

            info!("Connection upgraded to WS, starting a simple WS echo server now");

            // Now we have the TCP socket in a state where it can be operated as a WS connection
            // Run a simple WS echo server here

            let mut socket = conn.unbind()?;

            let mut buf = [0_u8; 8192];

            loop {
                let mut header = FrameHeader::recv(&mut socket)
                    .await
                    .map_err(WsHandlerError::Ws)?;
                let payload = header
                    .recv_payload(&mut socket, &mut buf)
                    .await
                    .map_err(WsHandlerError::Ws)?;

                match header.frame_type {
                    FrameType::Text(_) => {
                        info!(
                            "Got {header}, with payload \"{}\"",
                            core::str::from_utf8(payload).unwrap()
                        );
                    }
                    FrameType::Binary(_) => {
                        info!("Got {header}, with payload {payload:?}");
                    }
                    FrameType::Close => {
                        info!("Got {header}, client closed the connection cleanly");
                        break;
                    }
                    _ => {
                        info!("Got {header}");
                    }
                }

                // Echo it back now

                header.mask_key = None; // Servers never mask the payload

                if matches!(header.frame_type, FrameType::Ping) {
                    header.frame_type = FrameType::Pong;
                }

                info!("Echoing back as {header}");

                header.send(&mut socket).await.map_err(WsHandlerError::Ws)?;
                header
                    .send_payload(&mut socket, payload)
                    .await
                    .map_err(WsHandlerError::Ws)?;
            }
        }

        Ok(())
    }
}
```
