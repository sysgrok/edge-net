use core::fmt::{self, Debug};
use core::net::{SocketAddr, SocketAddrV4};

use crate as dhcp;

pub mod client;
pub mod server;

pub const DEFAULT_SERVER_PORT: u16 = 67;
pub const DEFAULT_CLIENT_PORT: u16 = 68;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error<E> {
    Io(E),
    Format(dhcp::Error),
}

pub type ErrorKind = Error<edge_nal::io::ErrorKind>;

impl<E> Error<E>
where
    E: edge_nal::io::Error,
{
    pub fn erase(&self) -> Error<edge_nal::io::ErrorKind> {
        match self {
            Self::Io(e) => Error::Io(e.kind()),
            Self::Format(e) => Error::Format(*e),
        }
    }
}

impl<E> From<dhcp::Error> for Error<E> {
    fn from(value: dhcp::Error) -> Self {
        Self::Format(value)
    }
}

impl<E> fmt::Display for Error<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "IO error: {err}"),
            Self::Format(err) => write!(f, "Format error: {err}"),
        }
    }
}

#[cfg(feature = "defmt")]
impl<E> defmt::Format for Error<E>
where
    E: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::Io(err) => defmt::write!(f, "IO error: {}", err),
            Self::Format(err) => defmt::write!(f, "Format error: {}", err),
        }
    }
}

impl<E> core::error::Error for Error<E> where E: core::error::Error {}
