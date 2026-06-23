//! Example of an HTTPS server.
//!
//! Uses `edge-nal-tls` to layer TLS (via `mbedtls-rs`) on top of `edge-nal`'s `TcpAccept`, and
//! `edge-http` to speak HTTP over the resulting TLS connection. This is the TLS counterpart of the
//! `http_server` example.
//!
//! The server answers a fixed text message to all HTTP GET / requests. Since the server certificate
//! is self-signed, the easiest way to test is with:
//! ```sh
//! curl -k https://localhost:8443/
//! ```

use core::convert::Infallible;
use core::fmt::{Debug, Display};
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use edge_http::io::server::{Connection, Handler, Server};
use edge_http::io::Error;
use edge_http::Method;
use edge_nal::{TcpBind, WithTimeout};

use edge_nal_tls::mbedtls::{Certificate, Credentials, PrivateKey, ServerSessionConfig, Tls, X509};
use edge_nal_tls::TlsAcceptor;

use embedded_io_async::{Read, Write};

use rand::{Rng, TryCryptoRng, TryRng};

use log::info;

/// The (self-signed) server certificate and its private key, in DER form.
const CERT: &[u8] = include_bytes!("certs/cert.der");
const KEY: &[u8] = include_bytes!("certs/key.der");

const PORT: u16 = 8443;

type TlsServer = Server<2, 2048, 20>;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let mut server = TlsServer::new();

    futures_lite::future::block_on(run(&mut server)).unwrap();
}

async fn run(server: &mut TlsServer) -> Result<(), anyhow::Error> {
    info!("Initializing TLS");

    let mut rng = StdRng;
    // SAFETY: `rng` is declared before `tls` and outlives it; `tls` is dropped at the end of this
    // scope and never leaked, so the borrow stays valid for the whole lifetime of the global RNG slot.
    let tls = unsafe { Tls::new_local_borrows(&mut rng) }.unwrap();

    let cert = Certificate::new_no_copy(CERT).unwrap();
    let config = ServerSessionConfig::new(Credentials {
        certificate: cert,
        private_key: PrivateKey::new(X509::DER(KEY), None).unwrap(),
    });

    // First, create a plain TCP acceptor on the `edge-nal` stack
    let tcp_acceptor = edge_nal_std::Stack::new()
        .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT))
        .await?;

    info!("Listening for incoming HTTPS connections on port {PORT}");

    // Next, layer the `mbedtls-rs` TLS stack on top of it
    let tls_acceptor = TlsAcceptor::new(tls.reference(), tcp_acceptor, &config);

    // Finally, run the HTTP server on top of the TLS acceptor (wrapped in a timeout, as TLS
    // shutdown can otherwise block indefinitely on a misbehaving peer)
    server
        .run(
            Some(15 * 1000),
            WithTimeout::new(15_000, tls_acceptor),
            HttpHandler,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {e:?}"))?;

    Ok(())
}

struct HttpHandler;

impl Handler for HttpHandler {
    type Error<E>
        = Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        info!("Got new connection");

        let headers = conn.headers()?;

        if headers.method != Method::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/" {
            conn.initiate_response(404, Some("Not Found"), &[]).await?;
        } else {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;

            conn.write_all(b"Hello from edge-http and edge-nal-tls!")
                .await?;
        }

        Ok(())
    }
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
