#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]

pub use edge_captive as captive;
pub use edge_dhcp as dhcp;
pub use edge_http as http;
pub use edge_mdns as mdns;
#[cfg(feature = "io")]
pub use edge_nal as nal;
#[cfg(feature = "embassy")]
pub use edge_nal_embassy as embassy;
#[cfg(feature = "std")]
pub use edge_nal_std as std;
pub use edge_raw as raw;
pub use edge_ws as ws;
