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
/// pool to erase the const-generics set on the Pool object type when used for TCP and UDP buffers.
///
/// To erase the type of the pool itself, these types use `&dyn DynPool<B>`
pub trait DynPool<B>: SealedDynPool<B> {}

impl<T, B> DynPool<B> for &T where T: DynPool<B> {}

mod sealed {
    use core::ptr::NonNull;

    /// The sealed trait variant of `DynPool`.
    pub trait SealedDynPool<B> {
        /// Allocate an object from the pool.
        ///
        /// Returns `None` if the pool is exhausted.
        fn alloc(&self) -> Option<B>;

        /// Free an object back to the pool.
        ///
        /// # Safety
        /// - `buffer_token` must be a pointer obtained from `alloc` that hasn't been freed yet.
        unsafe fn free(&self, buffer_token: NonNull<u8>);
    }

    impl<T, B> SealedDynPool<B> for &T
    where
        T: SealedDynPool<B>,
    {
        fn alloc(&self) -> Option<B> {
            (**self).alloc()
        }

        unsafe fn free(&self, buffer_token: NonNull<u8>) {
            (**self).free(buffer_token)
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

impl<T, const N: usize> Default for Pool<T, N> {
    fn default() -> Self {
        Self::new()
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

#[cfg(test)]
pub(crate) mod test {
    use core::task::Context;

    use embassy_net::driver::{Capabilities, Driver, HardwareAddress, LinkState, RxToken, TxToken};
    use embassy_net::{
        Config, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
    };

    use super::Pool;

    /// A no-op `defmt` logger, so that the tests link when the `defmt` feature is enabled.
    #[cfg(feature = "defmt")]
    mod defmt_logger {
        #[defmt::global_logger]
        struct Logger;

        unsafe impl defmt::Logger for Logger {
            fn acquire() {}
            unsafe fn flush() {}
            unsafe fn release() {}
            unsafe fn write(_bytes: &[u8]) {}
        }

        #[defmt::panic_handler]
        fn panic() -> ! {
            core::panic!("defmt panic")
        }

        defmt::timestamp!("{=u64:us}", 0);
    }

    /// A network driver that never sends or receives anything.
    ///
    /// Used to construct a `Stack` for the socket tests, which only exercise
    /// the socket buffers management and never actually communicate.
    pub(crate) struct DummyDriver;

    /// A token type that is never constructed, as `DummyDriver` never produces any tokens.
    pub(crate) enum Never {}

    impl RxToken for Never {
        fn consume<R, F>(self, _f: F) -> R
        where
            F: FnOnce(&mut [u8]) -> R,
        {
            match self {}
        }
    }

    impl TxToken for Never {
        fn consume<R, F>(self, _len: usize, _f: F) -> R
        where
            F: FnOnce(&mut [u8]) -> R,
        {
            match self {}
        }
    }

    impl Driver for DummyDriver {
        type RxToken<'a> = Never;
        type TxToken<'a> = Never;

        fn receive(&mut self, _cx: &mut Context) -> Option<(Never, Never)> {
            None
        }

        fn transmit(&mut self, _cx: &mut Context) -> Option<Never> {
            None
        }

        fn link_state(&mut self, _cx: &mut Context) -> LinkState {
            LinkState::Down
        }

        fn capabilities(&self) -> Capabilities {
            let mut caps = Capabilities::default();
            caps.max_transmission_unit = 1514;

            caps
        }

        fn hardware_address(&self) -> HardwareAddress {
            HardwareAddress::Ethernet([2, 0, 0, 0, 0, 1])
        }
    }

    /// Create a `Stack` backed by `DummyDriver`, for the socket tests.
    pub(crate) fn stack<const SOCK: usize>(
        resources: &mut StackResources<SOCK>,
    ) -> (Stack<'_>, Runner<'_, DummyDriver>) {
        embassy_net::new(
            DummyDriver,
            Config::ipv4_static(StaticConfigV4 {
                address: Ipv4Cidr::new(Ipv4Address::new(10, 0, 0, 1), 24),
                gateway: None,
                dns_servers: Default::default(),
            }),
            resources,
            0,
        )
    }

    #[test]
    fn pool_alloc_free() {
        let pool = Pool::<u32, 2>::new();

        let a = pool.alloc().unwrap();
        let b = pool.alloc().unwrap();
        assert!(pool.alloc().is_none());
        assert_ne!(a, b);

        // SAFETY: both slots are allocated and nothing else references them
        unsafe {
            a.as_ptr().write(1);
            b.as_ptr().write(2);
            assert_eq!(a.as_ptr().read(), 1);
            assert_eq!(b.as_ptr().read(), 2);

            pool.free(a);
        }

        // A freed slot is handed out again, with the other slot still allocated
        let c = pool.alloc().unwrap();
        assert_eq!(c, a);
        assert!(pool.alloc().is_none());

        // SAFETY: `b` and `c` are allocated
        unsafe {
            assert_eq!(b.as_ptr().read(), 2);

            pool.free(b);
            pool.free(c);
        }

        assert!(pool.alloc().is_some());
        assert!(pool.alloc().is_some());
        assert!(pool.alloc().is_none());
    }
}
