//! DHCP server example using raw sockets for full RFC 2131 compliance.
//!
//! This example demonstrates how to run a DHCP server using raw sockets, which provides
//! MAC-level addressing for better RFC 2131 compliance. This is useful in complex network
//! scenarios but requires root privileges.
//!
//! # Note
//! Run this example with `sudo` to be able to bind to the interface, as it uses raw sockets
//! which require root privileges:
//! ```sh
//! sudo cargo run --example dhcp_server_raw --features std
//! ```
//!
//! For most use cases, the regular UDP version (dhcp_server example) works fine and doesn't
//! require root privileges.

use core::net::{Ipv4Addr, SocketAddrV4};

use edge_dhcp::io::{self, DEFAULT_CLIENT_PORT, DEFAULT_SERVER_PORT};
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::RawBind;
use edge_raw::io::RawSocket2Udp;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    futures_lite::future::block_on(run(
        0, // The interface index of the interface (e.g. eno0) to use; run `ip addr` to see it
    ))
    .unwrap();
}

async fn run(if_index: u32) -> Result<(), anyhow::Error> {
    let stack = edge_nal_std::Interface::new(if_index);

    let mut buf = [0; 1500];

    let ip = Ipv4Addr::new(192, 168, 0, 1);

    let mut socket: RawSocket2Udp<_> = RawSocket2Udp::new(
        stack.bind().await?,
        Some(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )),
        Some(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_CLIENT_PORT,
        )),
        [0; 6],
    );

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
