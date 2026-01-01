# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-01-01
* Update the `embedded-io-async` and `embassy-time` dependencies

## [0.5.0] - 2025-01-15
* Updated `embassy-time` to v0.4

## [0.4.0] - 2025-01-02
* Proper TCP socket shutdown with a new `TcpShutdown` trait; Generic TCP timeout utils (#34)
* WithTimeout impl for TcpAccept; with_timeout now usable for any fallible future
* Option to erase the generics from the IO errors

## [0.3.0] - 2024-09-10
* First release (with version 0.3.0 to align with the other `edge-net` crates)
