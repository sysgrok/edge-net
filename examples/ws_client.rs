use core::net::SocketAddr;

use anyhow::bail;

use edge_http::io::client::Connection;
use edge_http::ws::{MAX_BASE64_KEY_LEN, MAX_BASE64_KEY_RESPONSE_LEN, NONCE_LEN};
use edge_nal::{AddrType, Dns, TcpConnect};
use edge_ws::{FrameHeader, FrameType};

use rand::RngCore;

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

    let mut rng_source = rand::rng();

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
