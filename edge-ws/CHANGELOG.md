# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2026-01-01
* Update the `embedded-io-async` dependency
* Remove the `embedded-svc` feature

## [0.6.0] - 2025-05-29
* Optional `defmt` support via two new features (one has to specify one, or the other, or neither, but not both):
  * `log` - uses the `log` crate for all logging
  * `defmt` - uses the `defmt` crate for all logging, and implements `defmt::Format` for all library types that otherwise implement `Debug` and/or `Display`
* Respect payload length of control messages

## [0.4.0] - 2025-01-02
* Option to erase the generics from the IO errors

## [0.3.0] - 2024-09-10
* Migrated to the `edge-nal` traits
* New method, `FrameHeader::mask_with` that takes a user-supplied mask
