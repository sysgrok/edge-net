use core::net::Ipv4Addr;

use edge_nal::{UdpReceive, UdpSend};

use self::dhcp::{Options, Packet};

pub use super::*;

/// Runs the provided DHCP server asynchronously using the supplied UDP socket and server options.
///
/// All incoming BOOTP requests are processed by updating the DHCP server's internal simple database of leases,
/// and by issuing replies.
///
/// Dropping this future is safe in that it won't remove the internal leases' database,
/// so users are free to drop the future in case they would like to take a snapshot of the leases or inspect them otherwise.
///
/// # RFC 2131 Compliance and Real-World Behavior
///
/// The server follows RFC 2131 Section 4.1 and real-world DHCP implementations (e.g., BusyBox udhcpd)
/// for determining reply destinations:
///
/// - If `giaddr` is non-zero: sends to relay agent at `giaddr` on port 67
/// - If `giaddr` is zero and `ciaddr` is non-zero and broadcast flag is not set: unicasts to `ciaddr` on port 68
/// - Otherwise (if `ciaddr` is zero OR broadcast flag is set): broadcasts to 255.255.255.255 on port 68
///
/// **Important**: The server does NOT unicast to `yiaddr` even when the broadcast flag is not set,
/// as the client may not yet have a UDP socket listening on that IP address. Only `ciaddr` (the client's
/// current IP from the request) is safe for unicast transmission.
///
/// # Socket Requirements
///
/// The UDP socket must be capable of sending and receiving broadcast UDP packets.
///
/// ## Regular UDP Sockets
///
/// The server works with regular UDP sockets (not requiring raw socket or MAC address control).
/// In this mode:
/// - Broadcast messages work correctly
/// - Unicast messages to `ciaddr` work when the client already has an IP configured
/// - This covers most common DHCP scenarios (DISCOVER/OFFER/REQUEST/ACK)
///
/// ## Raw Sockets (Optional, for Full RFC Compliance)
///
/// When using `RawSocket2Udp` (raw sockets with MAC address control):
/// - The MAC address is automatically captured from received packets
/// - Unicast messages are sent to the specific client MAC at the Ethernet layer
/// - This provides better behavior in complex network scenarios
/// - However, raw sockets are not required for basic DHCP server operation
pub async fn run<T, F, const N: usize>(
    server: &mut dhcp::server::Server<F, N>,
    server_options: &dhcp::server::ServerOptions<'_>,
    socket: &mut T,
    buf: &mut [u8],
) -> Result<(), Error<T::Error>>
where
    T: UdpReceive + UdpSend,
    F: FnMut() -> u64,
{
    info!(
        "Running DHCP server for addresses {}-{} with configuration {:?}",
        server.range_start, server.range_end, server_options
    );

    loop {
        let (len, remote) = socket.receive(buf).await.map_err(Error::Io)?;
        let packet = &buf[..len];

        let request = match Packet::decode(packet) {
            Ok(request) => request,
            Err(err) => {
                warn!("Decoding packet returned error: {:?}", err);
                continue;
            }
        };

        let mut opt_buf = Options::buf();

        if let Some(reply) = server.handle_request(&mut opt_buf, server_options, &request) {
            // Determine destination address according to RFC 2131 Section 4.1
            // and real-world DHCP server implementations (e.g., BusyBox udhcpd)
            let remote = if matches!(remote, SocketAddr::V4(_)) {
                let dest_ip = if !request.giaddr.is_unspecified() {
                    // 1. If giaddr is non-zero, send to relay agent
                    request.giaddr
                } else if !request.ciaddr.is_unspecified() && !request.broadcast {
                    // 2. Unicast to ciaddr only if BOTH conditions are met:
                    //    - giaddr is zero AND ciaddr is non-zero (client has existing IP)
                    //    - AND broadcast flag is not set
                    // NOTE: We do NOT unicast to yiaddr! The client may not have a UDP socket
                    // listening on yiaddr yet. Only ciaddr (from the client's request) is safe.
                    request.ciaddr
                } else {
                    // 3. Otherwise (ciaddr is zero OR broadcast flag is set), broadcast
                    // This ensures the client receives the message even if it doesn't yet have
                    // an IP address configured or cannot receive unicast packets.
                    Ipv4Addr::BROADCAST
                };

                let dest_port = if !request.giaddr.is_unspecified() {
                    // Send to relay agent on server port (67)
                    DEFAULT_SERVER_PORT
                } else {
                    // Send to client on client port (68) per RFC 2131 Section 4.1
                    DEFAULT_CLIENT_PORT
                };

                SocketAddr::V4(SocketAddrV4::new(dest_ip, dest_port))
            } else {
                remote
            };

            socket
                .send(remote, reply.encode(buf)?)
                .await
                .map_err(Error::Io)?;
        }
    }
}
