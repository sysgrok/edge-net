# edge-dhcp

[![CI](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

Async + `no_std` + no-alloc implementation of the DHCP protocol.

For other protocols, look at the [edge-net](https://github.com/ivmarkov/edge-net) aggregator crate documentation.

## Examples

### DHCP client

```rust
//! NOTE: Run this example with `sudo` to be able to bind to the interface, as it uses raw sockets which require root privileges.

use core::net::{Ipv4Addr, SocketAddrV4};

use edge_dhcp::client::Client;
use edge_dhcp::io::{client::Lease, DEFAULT_CLIENT_PORT, DEFAULT_SERVER_PORT};
use edge_nal::{MacAddr, RawBind};
use edge_raw::io::RawSocket2Udp;

use log::info;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    futures_lite::future::block_on(run(
        2, // The interface index of the interface (e.g. eno0) to use; run `ip addr` to see it
        [0x4c, 0xcc, 0x6a, 0xa2, 0x23, 0xf5], // Your MAC addr here; run `ip addr` to see it
    ))
    .unwrap();
}

async fn run(if_index: u32, if_mac: MacAddr) -> Result<(), anyhow::Error> {
    let mut client = Client::new(rand::rng(), if_mac);

    let stack = edge_nal_std::Interface::new(if_index);
    let mut buf = [0; 1500];

    loop {
        let mut socket: RawSocket2Udp<_> = RawSocket2Udp::new(
            stack.bind().await?,
            Some(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                DEFAULT_CLIENT_PORT,
            )),
            Some(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                DEFAULT_SERVER_PORT,
            )),
            [255; 6], // Broadcast
        );

        let (mut lease, options) = Lease::new(&mut client, &mut socket, &mut buf).await?;

        info!("Got lease {lease:?} with options {options:?}");

        info!("Entering an endless loop to keep the lease...");

        lease.keep(&mut client, &mut socket, &mut buf).await?;
    }
}
```

### DHCP server

```rust
//! NOTE: Run this example with `sudo` to be able to bind to the interface, as it uses raw sockets which require root privileges.

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
```
