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
/// Note that the UDP socket that the server takes need to be capable of sending and receiving broadcast UDP packets.
///
/// Furthermore, some DHCP clients do send DHCP REQUEST packets without the broadcast flag in the DHCP payload being set to true.
/// To support these clients, the socket needs to also be capable of sending packets with a broadcast IP destination address
/// - yet - with the destination *MAC* address in the Ethernet frame set to the MAC address of the DHCP client.
///
/// This is currently only possible with STD's BSD raw sockets' implementation. Unfortunately, `smoltcp` and thus `embassy-net`
/// do not have an equivalent (yet).
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
            let remote = if let SocketAddr::V4(socket) = remote {
                if request.broadcast || *socket.ip() == Ipv4Addr::UNSPECIFIED {
                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, socket.port()))
                } else {
                    remote
                }
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
