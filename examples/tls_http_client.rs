//! Example of an HTTPS client.
//!
//! Uses `edge-nal-tls` to layer TLS (via `mbedtls-rs`) on top of `edge-nal`'s `TcpConnect`, and
//! `edge-http` to speak HTTP over the resulting TLS connection. This is the TLS counterpart of the
//! `http_client` example.
//!
//! Connects to `https://httpbin.org/ip` and performs a simple HTTP 1.1 GET request.

use core::convert::Infallible;
use core::ffi::CStr;
use core::net::SocketAddr;

use edge_http::io::client::Connection;
use edge_http::Method;
use edge_nal::{AddrType, Dns};

use edge_nal_tls::mbedtls::{Certificate, ClientSessionConfig, Tls, X509};
use edge_nal_tls::TlsConnector;

use embedded_io_async::Read;

use rand::{Rng, TryCryptoRng, TryRng};

use log::info;

/// The trusted CA bundle used to verify the server certificate.
const CA_BUNDLE: &CStr = match CStr::from_bytes_with_nul(
    concat!(include_str!("certs/ca-bundle-small.pem"), "\0").as_bytes(),
) {
    Ok(bundle) => bundle,
    _ => panic!("CA bundle is not a valid text file"),
};

const SERVER_NAME: &CStr = c"httpbin.org";
const SERVER_PATH: &str = "/ip";

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let mut buf = [0_u8; 4096];

    futures_lite::future::block_on(run(&mut buf));
}

async fn run(buf: &mut [u8]) {
    info!("Initializing TLS");

    let mut rng = StdRng;
    // SAFETY: `rng` is declared before `tls` and outlives it; `tls` is dropped at the end of this
    // scope and never leaked, so the borrow stays valid for the whole lifetime of the global RNG slot.
    let tls = unsafe { Tls::new_local_borrows(&mut rng) }.unwrap();

    let stack = edge_nal_std::Stack::new();

    let server_name = SERVER_NAME.to_str().unwrap();

    info!("Resolving server {server_name}");

    let ip = stack
        .get_host_by_name(server_name, AddrType::IPv4)
        .await
        .unwrap();
    let addr = SocketAddr::new(ip, 443);

    info!("Using socket address {addr}");

    let config = ClientSessionConfig {
        ca_chain: Some(Certificate::new(X509::PEM(CA_BUNDLE)).unwrap()),
        server_name: Some(SERVER_NAME),
        ..ClientSessionConfig::new()
    };

    // Layer the TLS connector on top of the plain TCP `edge-nal` stack
    let connector = TlsConnector::new(tls.reference(), stack, &config);

    info!("Creating HTTPS connection");

    let mut conn = Connection::<_, 32>::new(buf, &connector, addr);

    info!("Requesting GET {SERVER_PATH} from server");

    conn.initiate_request(false, Method::Get, SERVER_PATH, &[("Host", server_name)])
        .await
        .unwrap();

    conn.initiate_response().await.unwrap();

    info!("Response headers: {}", conn.headers().unwrap());

    info!("Response body:\n>>>>>>>>");

    let mut body = [0_u8; 1024];
    loop {
        let len = conn.read(&mut body).await.unwrap();
        if len == 0 {
            break;
        }

        info!("{}", core::str::from_utf8(&body[..len]).unwrap_or("???"));
    }

    info!("<<<<<<<<");

    conn.close().await.unwrap();
}

/// A std, crypto-compliant random number generator (using the `rand` crate) for driving `mbedtls-rs`.
struct StdRng;

impl TryRng for StdRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(rand::rng().next_u32())
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(rand::rng().next_u64())
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        rand::rng().fill_bytes(dst);
        Ok(())
    }
}

impl TryCryptoRng for StdRng {}
