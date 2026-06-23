# edge-nal-tls

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-nal-tls.svg)

An implementation of [`edge-nal`](../edge-nal)'s TCP factory traits (`TcpAccept` / `TcpConnect`) and
socket traits (`Read` / `Write` / `Readable` / `TcpSplit` / `TcpShutdown`) over TLS, by layering
[`mbedtls-rs`](https://github.com/esp-rs/mbedtls-rs) on top of an existing `edge-nal` TCP transport.

This makes it possible to run any `edge-nal`-based protocol (e.g. `edge-http`) over a TLS connection,
regardless of the underlying TCP stack (`edge-nal-std`, `edge-nal-embassy`, ...).

## Usage

- `TlsConnector` wraps a `TcpConnect` transport and produces TLS client sockets.
- `TlsAcceptor` wraps a `TcpAccept` transport and produces TLS server sockets.

Both return a `TlsSocket`, a thin newtype wrapper around `mbedtls-rs`'s `Session` that implements the
`edge-nal` socket traits, so it can be used anywhere an `edge-nal` TCP socket is expected.

A single active `mbedtls::Tls` instance is required to drive the TLS stack; see the `mbedtls-rs`
documentation for details.
