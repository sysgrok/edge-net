# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

* Updated embassy-futures to 0.1.2

## [0.6.0] - 2025-05-29
* Optional `defmt` support via two new features (one has to specify one, or the other, or neither, but not both):
  * `log` - uses the `log` crate for all logging
  * `defmt` - uses the `defmt` crate for all logging, and implements `defmt::Format` for all library types that otherwise implement `Debug` and/or `Display`

## [0.5.0] - 2025-01-15
* Updated dependencies for compatibility with `embassy-time-driver` v0.2

## [0.4.0] - 2025-01-02
* Reduce logging level (#32)
* Support for Captive Portal URLs (#31)
* Option to erase the generics from the IO errors
* Make embassy-time optional

## [0.3.0] - 2024-09-10
* Migrated the client and the server to the `edge-nal` traits
* Migrated the server to only require `UdpSend` and `UdpReceive`, without the need to manipulate raw IP payloads anymore
* Raised MSRV to 1.77
