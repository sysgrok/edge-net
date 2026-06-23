# edge-nal-openthread

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-nal-openthread.svg)

An implementation of [`edge-nal`](../edge-nal)'s UDP factory traits (`UdpBind` / `UdpConnect`) and
socket traits (`UdpSend` / `UdpReceive` / `Readable` / `MulticastV4` / `MulticastV6` / `UdpSplit`)
over OpenThread (Thread), by wrapping the [`openthread`](https://github.com/esp-rs/openthread) crate.

This lets any `edge-nal`-based UDP protocol run over a Thread network.

## Usage

- `OtUdpStack` wraps an `openthread::OpenThread` instance and implements `UdpBind` / `UdpConnect`.
- It produces `OtUdpSocket`s, which implement the `edge-nal` UDP socket traits.

Both are thin newtype wrappers around the `openthread` types (the wrappers are needed because both
the `edge-nal` traits and the `openthread` types are foreign to this crate, which the orphan rule
does not allow to combine directly). The underlying `openthread` value remains reachable via
`OtUdpStack::openthread` / `OtUdpSocket::inner`.

> Note: only IPv6 addresses are supported (Thread is IPv6-only); multicast join/leave are no-ops.
