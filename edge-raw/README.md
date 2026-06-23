# edge-raw

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

Async + `no_std` + no-alloc implementation of IP and UDP packet creation and parsing.

The `edge_raw::io` module contains implementations of the `edge_nal::RawBind` trait, as well as of the `edge_nal::RawReceive` and `edge_nal::RawSend` traits.

These are useful in the context of protocols like DHCP, which - while working on top of UDP - need to be capable of receiving
and sending packets to peers that do not have an IP address assigned yet.

For other protocols, look at the [edge-net](https://github.com/sysgrok/edge-net) aggregator crate documentation.

## Examples

Look at the [edge-dhcp](../edge-dhcp) crate as to how to utilize the capabilities of `edge-raw`.
