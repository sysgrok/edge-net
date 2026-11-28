# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
* Unit tests for the host and service responders
* A fix for `DoubleEndedIterator` (#104)
* `Host` and `Service` records carry the cache-flush bit (RFC 6762 §10.2); other `HostAnswers` mark their unique records with the new `CLASS_IN_UNIQUE`
* Names are compressed in the messages that `HostAnswersMdnsHandler` and `HostQuestions::query` build
* `HostAnswersMdnsHandler` puts each record in a message once, though the answers visit it more than once (a host's address records from each of its services, the DNS-SD enumeration record from each service of a type) or several questions ask for it
* `HostAnswersMdnsHandler` adds to a reply only the additional records related to its answers (RFC 6763, section 12): the SRV and TXT records of the service instances that PTR answers name, and the A and AAAA records of the hosts that SRV records name. It added every A, AAAA, SRV and TXT record it had to a reply to any query, whenever it had a service
* `HostAnswersMdnsHandler` splits an announcement or a reply that doesn't fit in one message over several, and leaves out the additional records that don't fit. A reply to a legacy query is still one message, so its answers must fit in one
* Breaking: a new `MdnsResponse::Partial` variant: the first message of several, and the `Continuation` where the next starts. A new `MdnsHandler::handle_part` prepares the message at a `Continuation`; it defaults to `handle`'s one message. `Mdns` sends every message

## [0.8.0] - 2026-06-25
* Breaking: update the `edge-nal` dependency
* Breaking: update the `rand_core` dependency to 0.10
* Breaking: update the `domain` dependency to 0.12
* 
## [0.7.0] - 2026-01-01
* Update the `edge-nal` and `embassy-time` dependencies
* Update the `domain` dependency to 0.11
* Addressed #60

## [0.6.1] - 2025-08-14
* Fix HostAnswersMdnsHandler responding to any A/AAAA queries even without match

## [0.6.0] - 2025-05-29
* Optional `defmt` support via two new features (one has to specify one, or the other, or neither, but not both):
  * `log` - uses the `log` crate for all logging
  * `defmt` - uses the `defmt` crate for all logging, and implements `defmt::Format` for all library types that otherwise implement `Debug` and/or `Display`

## [0.5.0] - 2025-01-15
* Updated dependencies for compatibility with `embassy-time-driver` v0.2

## [0.4.0] - 2025-01-02
* Fix for #24 / avahi - always broadcast to any of the enabled muticast addresses, regardless how we were contacted with a query
* Support for one-shot queries
* Option to erase the generics from the IO errors
* Reduce logging level for the mDNS responder (#43)
* Provide an IPv4-only default socket for mdns (#51)
* wait_readable flag; waiting for the socket is now turned off by default due to suspicions that it does not work quite right with embassy-net; Only lock the send buffer once we received a packet

## [0.3.0] - 2024-09-10
Almost a complete rewrite:
* New query API (`Mdns::query`) complementing the responder / query answers' processing one (`Mdns::run`)
* `domain` API is now also a public API of `edge-mdns`, re-exported as `edge_mdns::domain`
* IO layer now uses the UDP traits from `edge-net`
* Traits:
  * `MdnsHandler` - abstracts the overall processing of an incoming mDNS message
  * `HostAnswers` - abstracts the generation of answers to peer queries (implemented by the pre-existing `Host` and `Service` struct types)
  * `HostQuestions` - , `PeerAnswers`
Smaller items:
* Raised the `domain` dependency to 0.10
* Optimized memory usage by avoiding on-stack allocation of large `heapless::String`s
* IO layer of `edge-mdns` can now share its buffers with other code (see the `BufferAccess` trait)
* Raised MSRV to 1.77
