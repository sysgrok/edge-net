use core::fmt::Display;
use core::net::IpAddr;

use edge_nal::AddrType;

use embassy_net::dns::{DnsQueryType, Error};
use embassy_net::Stack;

use embedded_io_async::ErrorKind;

/// A type that implements the `Dns` trait from `edge-nal`.
/// It uses the DNS resolver from the provided Embassy networking stack.
///
/// The type is `Copy` and `Clone`, so it can be easily passed around.
#[derive(Copy, Clone)]
pub struct Dns<'a> {
    stack: Stack<'a>,
}

impl<'a> Dns<'a> {
    /// Create a new `Dns` instance for the provided Embassy networking stack
    ///
    /// NOTE: If using DHCP, make sure it has reconfigured the stack to ensure the DNS servers are updated
    pub fn new(stack: Stack<'a>) -> Self {
        Self { stack }
    }
}

impl edge_nal::Dns for Dns<'_> {
    type Error = DnsError;

    async fn get_host_by_name(
        &self,
        host: &str,
        addr_type: AddrType,
    ) -> Result<IpAddr, Self::Error> {
        let qtype = match addr_type {
            AddrType::IPv6 => DnsQueryType::Aaaa,
            _ => DnsQueryType::A,
        };
        let addrs = self.stack.dns_query(host, qtype).await?;
        if let Some(first) = addrs.first() {
            Ok((*first).into())
        } else {
            Err(Error::Failed.into())
        }
    }

    async fn get_host_by_address(
        &self,
        _addr: IpAddr,
        _result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        todo!()
    }
}

/// DNS error type
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DnsError(Error);

impl From<Error> for DnsError {
    fn from(e: Error) -> Self {
        DnsError(e)
    }
}

impl Display for DnsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DNS error: {:?}", self.0)
    }
}

impl core::error::Error for DnsError {}

impl embedded_io_async::Error for DnsError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}
