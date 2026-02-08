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
/// # RFC 2131 Compliance
///
/// The server follows RFC 2131 Section 4.1 for determining reply destinations:
/// - If `giaddr` is non-zero: sends to relay agent at `giaddr` on port 67
/// - If `giaddr` is zero and `ciaddr` is non-zero: unicasts to `ciaddr` on port 68
/// - If both are zero and broadcast flag is set: broadcasts to 255.255.255.255 on port 68
/// - If both are zero and broadcast flag is not set: unicasts to `yiaddr` on port 68
///
/// # Socket Requirements
///
/// Note that the UDP socket must be capable of sending and receiving broadcast UDP packets.
///
/// Furthermore, for the last case (unicast to `yiaddr` when broadcast flag is not set), the socket
/// needs to be capable of sending packets with a unicast IP destination address (`yiaddr`) yet with
/// the destination *MAC* address in the Ethernet frame set to the MAC address of the DHCP client.
/// This is currently only possible with STD's BSD raw sockets' implementation. Unfortunately, `smoltcp`
/// and thus `embassy-net` do not have an equivalent (yet).
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
            let remote = if let SocketAddr::V4(socket) = remote {
                let dest_ip = if !request.giaddr.is_unspecified() {
                    // 1. If giaddr is non-zero, send to relay agent
                    request.giaddr
                } else if !request.ciaddr.is_unspecified() {
                    // 2. If giaddr is zero and ciaddr is non-zero, unicast to ciaddr
                    request.ciaddr
                } else if request.broadcast {
                    // 3. If giaddr and ciaddr are zero and broadcast flag is set, broadcast
                    Ipv4Addr::BROADCAST
                } else if !reply.yiaddr.is_unspecified() {
                    // 4. If giaddr and ciaddr are zero and broadcast flag is not set,
                    //    unicast to yiaddr (the IP being offered/acknowledged)
                    reply.yiaddr
                } else {
                    // Fallback: If yiaddr is also unspecified, broadcast
                    Ipv4Addr::BROADCAST
                };

                let dest_port = if !request.giaddr.is_unspecified() {
                    // Send to relay agent on server port (67)
                    DEFAULT_SERVER_PORT
                } else {
                    // Send to client on client port (68)
                    socket.port()
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
