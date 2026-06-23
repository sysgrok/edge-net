# edge-nal-std

[![CI](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml/badge.svg)](https://github.com/sysgrok/edge-net/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/edge-net.svg)
[![Documentation](https://docs.rs/edge-net/badge.svg)](https://docs.rs/edge-net)

An implementation of the `edge-nal` traits for the Rust Standard Library.

## Implemented Traits

All.

## Implementation Details

The implementation is based on the minimalistic [async-io](https://github.com/smol-rs/async-io) crate from the [smol](https://github.com/smol-rs/smol) async echosystem.

Works out of the box on a variety of operating systems, including [Espressif's ESP IDF](https://github.com/espressif/esp-idf).
