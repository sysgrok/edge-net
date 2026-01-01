# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0] - 2026-01-01
* Update the `edge-nal` and `embedded-io-async` dependencies

## [0.7.0] - 2025-11-23
* Erase the const generics from the edge-nal-embassy types #78
* Make `Tcp`, `Udp` and `Dns` `Copy`
* Change the TX_SZ and RX_SZ defaults for UDP buffers to 1472 bytes which is a bit smaller yet enough if the overall Ethernet MTU is 1500 bytes (which it typically is; Ethernet MTU might be lower, but is rarely higher)

## [0.6.0] - 2025-05-29
* Optional `defmt` support via two new features (one has to specify one, or the other, or neither, but not both):
  * `log` - uses the `log` crate for all logging
  * `defmt` - uses the `defmt` crate for all logging, and implements `defmt::Format` for all library types that otherwise implement `Debug` and/or `Display`
* Updated to `embassy-net` 0.7
* Re-export all `embassy-net` features as `edge-nal-embassy` features; `all` feature that enables all features of `embassy-net`

## [0.5.0] - 2025-01-15
* Updated dependencies for compatibility with `embassy-time-driver` v0.2

## [0.4.1] - 2025-01-05
* Fix regression: ability to UDP/TCP bind to socket 0.0.0.0

## [0.4.0] - 2024-01-02
* Proper TCP socket shutdown; Generic TCP timeout utils; built-in HTTP server timeouts; update docu (#34)
* fix a typo (#44)
* Document the N generic for Udp as done for Tcp (#47)
* Update to embassy-net 0.5 (#50)

## [0.3.0] - 2024-09-10
* First release (with version 0.3.0 to align with the other `edge-net` crates)
