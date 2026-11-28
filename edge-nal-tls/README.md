# edge-nal-tls

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-nal-tls.svg)

An implementation of [`edge-nal`](../edge-nal)'s TCP factory traits (`TcpAccept` / `TcpConnect`) and
socket traits (`Read` / `Write` / `Readable` / `TcpSplit` / `TcpShutdown`) over TLS, by layering
[`mbedtls-rs`](https://github.com/esp-rs/mbedtls-rs) on top of an existing `edge-nal` TCP transport.

This makes it possible to run any `edge-nal`-based protocol (e.g. `edge-http`) over a TLS connection,
regardless of the underlying TCP stack (`edge-nal-std`, `edge-nal-embassy`, ...).

With the `dtls` feature, the crate also implements `edge-nal`'s UDP socket traits (`UdpReceive` / `UdpSend` /
`Readable` / `UdpSocket`) over DTLS, by layering `mbedtls-rs` on top of an existing `edge-nal` UDP socket
(`edge-nal-std`, `edge-nal-embassy`, `edge-nal-openthread`, ...).

## Usage

- `TlsConnector` wraps a `TcpConnect` transport and produces TLS client sockets.
- `TlsAcceptor` wraps a `TcpAccept` transport and produces TLS server sockets.

Both return a `TlsSocket`, a thin newtype wrapper around `mbedtls-rs`'s `Session` that implements the
`edge-nal` socket traits, so it can be used anywhere an `edge-nal` TCP socket is expected.

A single active `mbedtls::Tls` instance is required to drive the TLS stack; see the `mbedtls-rs`
documentation for details.

## DTLS

- `UdpDatagram` wraps an `edge-nal` UDP socket (bound or connected) and the address of the peer. It is the
  transport of an `mbedtls-rs` DTLS session (`mbedtls::dtls::Session`). It sends each datagram to the peer,
  and drops the datagrams that come from other addresses.
- `DtlsSocket` wraps such a session and implements the `edge-nal` UDP socket traits, as a UDP socket
  connected to the peer.

```rust
let socket = stack.bind(local).await?;

let mut socket = DtlsSocket::new(mbedtls::dtls::Session::new(
    tls.reference(),
    UdpDatagram::new(socket, server),
    mbedtls::dtls::EmbassyClock,
    &mbedtls::SessionConfig::Client(config),
    &mbedtls::dtls::DtlsConfig::new(),
)?);

socket.send(server, b"hello").await?;
```

Notes:
- Unlike with a plain UDP socket, `DtlsSocket::receive` does not drop the data of a record that does not fit
  in the buffer: the next `receive` returns it.
- A DTLS server needs the address of its client before the handshake, because it checks it with a cookie
  first (`mbedtls::dtls::CookieCheck`). `UdpDatagram` does not learn this address from the first datagram.
- On ESP-IDF, the `dtls` feature also needs `CONFIG_MBEDTLS_SSL_PROTO_DTLS`.
