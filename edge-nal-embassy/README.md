# edge-nal-embassy

[![CI](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/ivmarkov/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

A bare-metal implementation of `edge-nal` based on the [embassy-net](https://crates.io/crates/embassy-net) crate - the networking stack of the Embassy ecosystem.

## Implemented Traits

### TCP

All traits.

### UDP

* All traits except `UdpConnect`.

### Raw sockets

All traits.
