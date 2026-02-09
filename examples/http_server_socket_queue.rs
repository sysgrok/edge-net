use core::fmt::{Debug, Display};

use edge_http::io::server::{Connection, DefaultServer, Handler};
use edge_http::io::Error;
use edge_http::Method;
use edge_nal::TcpBind;

use embedded_io_async::{Read, Write};

use log::info;

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let mut server = DefaultServer::new();

    futures_lite::future::block_on(run(&mut server)).unwrap();
}

pub async fn run(server: &mut DefaultServer) -> Result<(), anyhow::Error> {
    let addr = "0.0.0.0:8881";

    info!("Running HTTP server with socket queue on {addr}");
    info!("This architecture decouples connection acceptance from HTTP processing");
    info!("Connections can be accepted even when all workers are busy");

    let acceptor = edge_nal_std::Stack::new()
        .bind(addr.parse().unwrap())
        .await?;

    // Use the new socket queue architecture for better connection handling
    // Q=8: Number of sockets that can be allocated (acceptor + worker tasks)
    server
        .run_with_socket_queue::<_, _, 8>(None, acceptor, HttpHandler)
        .await?;

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
        let headers = conn.headers()?;

        if headers.method != Method::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/" {
            conn.initiate_response(404, Some("Not Found"), &[]).await?;
        } else {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;

            conn.write_all(b"Hello world!").await?;
        }

        Ok(())
    }
}
