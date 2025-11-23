# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.1] - 2025-11-23
* Document the API
* Make `Stack` `Copy`

## [0.5.0] - 2025-01-15
* Updated dependencies for compatibility with `embassy-time-driver` v0.2

## [0.4.0] - 2025-01-02
* Proper TCP socket shutdown; Generic TCP timeout utils; built-in HTTP server timeouts; update docu (#34)
* Fix forgotten ref to async-io
* Clone for the STD stack
* For now assume the raw packet API is only usable on Linux

## [0.3.0] - 2024-09-10
* Crate name changed from `edge-std-nal-async` to just `edge-nal-std`
* Support for `embedded-nal-async` removed as the traits in that crate are too limited
* STD support is now targetting the traits of our own "nal" crate - `edge-nal`
* STD support can optionally use the `async-io-mini` crate instead of the default `async-io`

## [0.2.0] - 2024-02-01
* Do not compile the `RawSocket` implementation for the ESP IDF, as it is missing the `sockaddr_ll` structure in `libc`
* Retire `StdTcpSocketRef` in favor of simply using `&StdTcpSocket`
* ESP-IDF bugfix: The `TcpAccept` implementation was incorrect, because lwIP - unfortunately - does not support `select`-ing on server-side listening sockets
