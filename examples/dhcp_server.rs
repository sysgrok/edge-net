//! DHCP server example using regular UDP sockets.
//!
//! This example demonstrates how to run a DHCP server using standard UDP sockets
//! without requiring raw socket access or root privileges.
//!
//! The server will listen on port 67 and respond to DHCP requests from clients.
//!
//! # Note
//! For better RFC 2131 compliance with MAC-level addressing, you can use raw sockets
//! (requires root privileges). See the dhcp_server_raw example for that approach.

use core::net::{Ipv4Addr, SocketAddr};

use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::UdpBind;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    futures_lite::future::block_on(run()).unwrap();
}

async fn run() -> Result<(), anyhow::Error> {
    let stack = edge_nal_std::Stack::new();

    let mut buf = [0; 1500];

    let ip = Ipv4Addr::new(192, 168, 0, 1);

    // Bind to the DHCP server port (67) on all interfaces
    // The socket will have broadcast enabled automatically
    let mut socket = stack
        .bind(SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await?;

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    io::server::run(
        &mut Server::<_, 64>::new_with_et(ip), // Will give IP addresses in the range 192.168.0.50 - 192.168.0.200, subnet 255.255.255.0
        &ServerOptions::new(ip, Some(&mut gw_buf)),
        &mut socket,
        &mut buf,
    )
    .await?;

    Ok(())
}
