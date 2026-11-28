//! DTLS sessions over `edge-nal-std` UDP sockets on the loopback interface.

use core::convert::Infallible;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use std::sync::Mutex;

use edge_nal::{Readable, UdpBind, UdpReceive, UdpSend};

use edge_nal_tls::mbedtls::dtls::{
    CookieCheck, Cookies, Datagram, DtlsConfig, EmbassyClock, Session,
};
use edge_nal_tls::mbedtls::{
    Authentication, ClientSessionConfig, ServerSessionConfig, SessionConfig, SessionError, Tls,
};
use edge_nal_tls::{DtlsSocket, UdpDatagram};

use embassy_futures::join::join;
use embassy_time::{with_timeout, Duration};
use futures_lite::future::block_on;
use rand::{Rng, TryCryptoRng, TryRng};

/// Any free port on the loopback interface.
const LOOPBACK: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

const IDENTITY: &[u8] = b"edge-nal-tls";
const KEY: &[u8] = &[0x11; 16];

/// How long a test may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `mbedtls-rs` has one `Tls` instance at a time.
static SERIAL: Mutex<()> = Mutex::new(());

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

#[test]
fn ping_pong() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());

    block_on(async {
        let mut rng = StdRng;
        // SAFETY: `rng` outlives `tls`, which is not leaked.
        let tls = unsafe { Tls::new_local_borrows(&mut rng) }.unwrap();

        let stack = edge_nal_std::Stack::new();
        let client_socket = stack.bind(LOOPBACK).await.unwrap();
        let server_socket = stack.bind(LOOPBACK).await.unwrap();
        let client_addr = client_socket.get_ref().local_addr().unwrap();
        let server_addr = server_socket.get_ref().local_addr().unwrap();

        let psk = Authentication::Psk {
            identity: IDENTITY,
            key: KEY,
        };

        let mut client = DtlsSocket::new(
            Session::new(
                tls.reference(),
                UdpDatagram::new(client_socket, server_addr),
                EmbassyClock,
                &SessionConfig::Client(ClientSessionConfig {
                    authentication: Some(psk.clone()),
                    ..ClientSessionConfig::new()
                }),
                &DtlsConfig::new(),
            )
            .unwrap(),
        );

        // The server checks its client's address with a cookie, so the client sends its
        // `ClientHello` twice
        let cookies = Cookies::new(tls.reference()).unwrap();
        let client_id = client_addr.to_string();

        let mut server = DtlsSocket::new(
            Session::new(
                tls.reference(),
                UdpDatagram::new(server_socket, client_addr),
                EmbassyClock,
                &SessionConfig::Server(ServerSessionConfig::new(psk)),
                &DtlsConfig {
                    cookie_check: Some(CookieCheck::Verify {
                        cookies: &cookies,
                        client: client_id.as_bytes(),
                    }),
                    ..DtlsConfig::new()
                },
            )
            .unwrap(),
        );

        let client = async {
            let mut buf = [0; 16];

            client.send(server_addr, b"ping").await?;
            client.readable().await?;
            let (len, from) = client.receive(&mut buf).await?;
            client.session_mut().close().await?;

            Ok::<_, SessionError>((buf[..len].to_vec(), from))
        };

        let server = async {
            let mut start = [0; 2];
            let mut rest = [0; 16];

            server.readable().await?;
            // The rest of the record is left for the next `receive`
            let (_, from) = server.receive(&mut start).await?;
            let (len, _) = server.receive(&mut rest).await?;
            server.send(client_addr, b"pong").await?;
            // The client has closed the session
            let (closed, _) = server.receive(&mut rest).await?;

            Ok::<_, SessionError>(([&start[..], &rest[..len]].concat(), from, closed))
        };

        let (client, server) = with_timeout(TIMEOUT, join(client, server)).await.unwrap();

        assert_eq!(client.unwrap(), (b"pong".to_vec(), server_addr));
        assert_eq!(server.unwrap(), (b"ping".to_vec(), client_addr, 0));
    });
}

#[test]
fn drops_foreign_and_empty_datagrams() {
    block_on(async {
        let stack = edge_nal_std::Stack::new();
        let socket = stack.bind(LOOPBACK).await.unwrap();
        let mut peer = stack.bind(LOOPBACK).await.unwrap();
        let mut stranger = stack.bind(LOOPBACK).await.unwrap();
        let addr = socket.get_ref().local_addr().unwrap();
        let peer_addr = peer.get_ref().local_addr().unwrap();

        stranger.send(addr, b"stranger").await.unwrap();
        // An empty datagram would end the DTLS session
        peer.send(addr, b"").await.unwrap();
        peer.send(addr, b"peer").await.unwrap();

        let mut socket = UdpDatagram::new(socket, peer_addr);
        let mut buf = [0; 16];

        let len = with_timeout(TIMEOUT, socket.recv(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..len], b"peer");

        socket.send(b"back").await.unwrap();

        let (len, from) = with_timeout(TIMEOUT, peer.receive(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((&buf[..len], from), (&b"back"[..], addr));
    });
}
