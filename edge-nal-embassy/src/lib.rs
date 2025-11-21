#![no_std]
#![allow(async_fn_in_trait)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::cell::{Cell, UnsafeCell};
use core::mem::MaybeUninit;
use core::net::{IpAddr, SocketAddr};
use core::ptr::NonNull;

use embassy_net::{IpAddress, IpEndpoint, IpListenEndpoint};

#[cfg(feature = "dns")]
pub use dns::*;
#[cfg(feature = "tcp")]
pub use tcp::*;
#[cfg(feature = "udp")]
pub use udp::*;

use crate::sealed::SealedDynPool;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "dns")]
mod dns;
#[cfg(feature = "tcp")]
mod tcp;
#[cfg(feature = "udp")]
mod udp;

/// A const-generics-erased trait variant of `Pool`
///
/// Allows for types like `Tcp`, `TcpSocket`, `Udp` and `UdpSocket` that do reference the
/// pool to erase the const-genenerics set on the Pool object type when used for TCP and UDP buffers.
///
/// To erase the type of the pool itself, these types use `&dyn DynPool<B>`
pub trait DynPool<B>: SealedDynPool<B> {}

impl<T, B> DynPool<B> for &T where T: DynPool<B> {}

mod sealed {
    /// The sealed trait variant of `DynPool`.
    pub trait SealedDynPool<B> {
        /// Allocate an object from the pool.
        ///
        /// Returns `None` if the pool is exhausted.
        fn alloc(&self) -> Option<B>;

        /// Free an object back to the pool.
        fn free(&self, buffer: B);
    }

    impl<T, B> SealedDynPool<B> for &T
    where
        T: SealedDynPool<B>,
    {
        fn alloc(&self) -> Option<B> {
            (**self).alloc()
        }

        fn free(&self, buffer: B) {
            (**self).free(buffer)
        }
    }
}

/// A simple fixed-size pool allocator for `T`.
pub struct Pool<T, const N: usize> {
    used: [Cell<bool>; N],
    data: [UnsafeCell<MaybeUninit<T>>; N],
}

impl<T, const N: usize> Pool<T, N> {
    #[allow(clippy::declare_interior_mutable_const)]
    const VALUE: Cell<bool> = Cell::new(false);
    #[allow(clippy::declare_interior_mutable_const)]
    const UNINIT: UnsafeCell<MaybeUninit<T>> = UnsafeCell::new(MaybeUninit::uninit());

    /// Create a new pool.
    pub const fn new() -> Self {
        Self {
            used: [Self::VALUE; N],
            data: [Self::UNINIT; N],
        }
    }
}

impl<T, const N: usize> Pool<T, N> {
    /// Allocate an object from the pool.
    ///
    /// # Returns
    /// - `Some(NonNull<T>)` if an object was successfully allocated.
    /// - `None` if the pool is exhausted.
    fn alloc(&self) -> Option<NonNull<T>> {
        for n in 0..N {
            // this can't race because Pool is not Sync.
            if !self.used[n].get() {
                self.used[n].set(true);
                let p = self.data[n].get() as *mut T;
                return Some(unsafe { NonNull::new_unchecked(p) });
            }
        }
        None
    }

    /// Free an object back to the pool.
    ///
    /// Safety: p must be a pointer obtained from `alloc` that hasn't been freed yet.
    ///
    /// # Arguments
    /// - `p`: A pointer to the object to free.
    unsafe fn free(&self, p: NonNull<T>) {
        let origin = self.data.as_ptr() as *mut T;
        let n = p.as_ptr().offset_from(origin);
        assert!(n >= 0);
        assert!((n as usize) < N);
        self.used[n as usize].set(false);
    }
}

/// Convert an embassy-net `IpEndpoint` to a standard library `SocketAddr`.
pub(crate) fn to_net_socket(socket: IpEndpoint) -> SocketAddr {
    SocketAddr::new(socket.addr.into(), socket.port)
}

/// Convert an embassy-net `IpListenEndpoint` to a standard library `SocketAddr`.
pub(crate) fn to_emb_socket(socket: SocketAddr) -> Option<IpEndpoint> {
    Some(IpEndpoint {
        addr: to_emb_addr(socket.ip())?,
        port: socket.port(),
    })
}

/// Convert a standard library `SocketAddr` to an embassy-net `IpListenEndpoint`.
pub(crate) fn to_emb_bind_socket(socket: SocketAddr) -> Option<IpListenEndpoint> {
    let addr = if socket.ip().is_unspecified() {
        None
    } else {
        Some(to_emb_addr(socket.ip())?)
    };

    Some(IpListenEndpoint {
        addr,
        port: socket.port(),
    })
}

/// Convert an embassy-net `IpAddress` to a standard library `IpAddr`.
pub(crate) fn to_emb_addr(addr: IpAddr) -> Option<IpAddress> {
    match addr {
        #[cfg(feature = "proto-ipv4")]
        IpAddr::V4(addr) => Some(addr.into()),
        #[cfg(feature = "proto-ipv6")]
        IpAddr::V6(addr) => Some(addr.into()),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}
