# edge-net

[![CI](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

This crate ships async + `no_std` + no-alloc implementations of various network protocols.

Suitable for microcontrollers and embedded systems in general.

## Supported protocols

* [HTTP client and server](edge-http)
* [Websocket send/receive](edge-ws)
* [DNS Captive Portal](edge-captive)
* [mDNS responder](edge-mdns)
* [DHCP cient and server](edge-dhcp)
* [Raw IP & UDP packet send/receive](edge-raw) (useful in combination with the DHCP client and server)
* [TCP, UDP and raw sockets](edge-nal)

## Supported platforms

* [The Rust Standard library](edge-nal-std)
* [The networking stack of Embassy](edge-nal-embassy)
* Any other platform, as long as you implement (a subset of) [edge-nal](edge-nal)
  * The necessary minimum being the `Read` / `Write` traits from [embedded_io_async](https://crates.io/crates/embedded-io-async/0.5.0) - for modeling TCP sockets - and `UdpReceive` / `UdpSend` from [edge-nal](edge-nal) - for modeling UDP sockets
  * Most crates ([edge-captive](edge-captive), [edge-dhcp](edge-dhcp), [edge-ws](edge-ws), [edge-raw](edge-raw)) also provide a compute-only subset that does not need [embedded-io-async](https://crates.io/crates/embedded-io-async/0.5.0) or [edge-nal](edge-nal) traits

**PRs welcome!**
