# edge-nal

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

Hosts a bunch of networking (UDP, TCP and raw ethernet) traits.

## Differences with [embedded-nal-async](https://github.com/rust-embedded-community/embedded-nal/tree/master/embedded-nal-async)

### TCP

* Factory traits for the creation of TCP server sockets - `TcpBind` and `TcpAccept`. `embedded-nal-async` only has `TcpConnect`
* Splittable sockets with `TcpSplit` (can be optionally implemented by `TcpConnect` and `TcpAccept`)
* Socket shutdown with `TcpShutdown`

### UDP

* Separate `UdpSend` and `UdpReceive` traits for modeling the sending / receiving functinality of a UDP socket. Necessary for protocols that need UDP socket splitting, like mDNS responder
* Binding to a UDP socket and connecting to a UDP socket modeled with separate traits - `UdpBind` and `UdpConnect`, as not all platforms currently have capabilities to connect to a UDP socket (i.e. the networking stack of Embassy)
* Returning the local address of a UDP socket bind / connect operation is not supported, as not all platforms currently have this capability (i.e. the networking stack of Embassy)
* "Unbound" UDP sockets are currently not supported, as not all platforms have these capabilities (i.e. the networking stack of Embassy). Also, I've yet to find a good use case for these.
* Splittable sockets with `UdpSplit`
* `MulticastV4` and `MulticastV6` traits for joining / leaving IPv4 and IPv6 multicast groups (can be optionally implemented by `UdpConnect` and `UdpBind`)
* `Readable` trait for waiting until a socket becomes readable

## Justification

These traits are necessary to unlock the full functionality of some crates in `edge-net`, which is not possible with the current traits of `embedded-nal-async`. 

Namely:
* [edge-mdns](../edge-mdns) - needs UDP multicast capabilities as well as socket splitting
* [edge-dhcp](../edge-dhcp) - needs raw ethernet socket capabilities or at least sending/receiving UDP packets to/from peers identified by their MAC addresses rather than by their IP addresses
* [edge-http](../edge-http) - (full server only) needs a way to bind to a server-side TCP socket
* [edge-ws](../edge-ws) - Most WebSocket use cases do require a splittable TCP socket (separate read and write halves)

## Traits

### TCP

* [TcpSplit](src/stack/tcp.rs)
  * A trait that - when implemented on a TCP socket - allows for splitting the send and receive halves of the socket for full-duplex functionality
* [TcpConnect](src/stack/tcp.rs)
  * Client-side TCP socket factory similar in spirit to STD's `std::net::TcpListener::connect` method
* [TcpBind](src/stack/tcp.rs)
  * Server-side TCP socket factory similar in spirit to STD's `std::net::TcpListener::bind` method and `std::net::TcpListener` struct
* [TcpAccept](src/stack/tcp.rs)
  * The acceptor of the server-side TCP socket factory similar in spirit to STD's `std::net::TcpListener::bind` method and `std::net::TcpListener` struct

### UDP

* [UdpReceive](src/udp.rs)
  * The receiver half of a UDP socket
* [UdpSend](src/udp.rs)
  * The sender half of a UDP socket
* [UdpSplit](src/stack/udp.rs)
  * A trait that - when implemented on a UDP socket - allows for splitting the send and receive halves of the socket for full-duplex functionality
* [UdpBind](src/stack/udp.rs)
  * Udp socket factory similar in spirit to STD's `std::net::UdpSocket::bind` method
* [UdpConnect](src/stack/udp.rs)
  * Udp socket factory similar in spirit to STD's `std::net::UdpSocket::connect` method
* [Multicastv4 and MulticastV6](src/multicast.rs)
  * Extra traits for UDP sockets allowing subscription to multicast groups
* [Readable](src/readable.rs)
  * Extra trait for UDP, TCP and raw sockets allowing one to wait until the socket becomes readable

### Traits for sending/receiving raw ethernet payloads (a.k.a. raw sockets)

* [RawReceive](src/raw.rs)
  * The receiver half of a raw socket
* [RawSend](src/raw.rs)
  * The sender half of a raw socket
* [RawSplit](src/stack/raw.rs)
  * A trait that - when implemented on a raw socket - allows for splitting the send and receive halves of the socket for full-duplex functionality
* [RawBind](src/stack/raw.rs)
  * A raw socket factory
