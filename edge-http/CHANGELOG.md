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

## [0.5.1] - 2025-02-02
* Fix multiple websocket-related connection issues (#58)

## [0.5.0] - 2025-01-15
* Updated dependencies for compatibility with `embassy-time-driver` v0.2

## [0.4.0] - 2025-01-02
* Connection type support (#33)
* Proper TCP socket shutdown; Generic TCP timeout utils; built-in HTTP server timeouts; update docu (#34)
* Always send a SP after status code, even if no reason is given (#36)
* edge-http: make fields in {Req,Resp}Headers non-optional (#37)
* Combine Handler and TaskHandler; eradicate all explicit timeouts, now that both TcpAccept and Handler are implemented for WithTimeout
* Fix memory consumption when using a handler with a timeout
* (edge-http) Server non-static handler (#40)
* Option to erase the generics from the IO errors
* HTTP client: Close method for connections

## [0.3.0] - 2024-09-10
* Migrated the client and the server to the `edge-nal` traits
* Fixed a nasty bug where when multiple HTTP requests were carried over a single TCP connection, in certain cases the server was "eating" into the data of the next HTTP request
* #20 - Removed a misleading warning log "Connection(IncompleteHeaders)"

## [0.2.1] - 2024-02-01
* Fixed a wrong header name which caused WS client socket upgrade to fail

## [0.2.0] - 2024-02-01
* Remove unnecessary lifetimes when implementing the `embedded-svc` traits
* Server: new trait, `TaskHandler` which has an extra `task_id` parameter of type `usize`. This allows the request handling code to take advantage of the fact that - since the number of handlers when running a `Server` instance is fixed - it can store data related to handlers in a simple static array of the same size as the number of handlers that the server is running
* Breaking change: structures `Server` and `ServerBuffers` united, because `Server` was actually stateless. Turbofish syntax for specifying max number of HTTP headers and queue size is no longer necessary
* Breaking change: introduce an optional timeout for HTTP server connections and for the server itself
* Breaking change: remove the `const W: usize` parameter from the `Server` struct, as the accept queue is no longer necessary (using an async mutex now internally)
* Fix a bug where the Websockets' `Sec-Key-Accept` header was computed incorrectly
* Implement `Sec-Key-Accept` header validation in the HTTP client
* Breaking change: `UpgradeError::SecKeyTooLong` removed as it is no longer used
