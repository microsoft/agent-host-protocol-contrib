use std::io;
use std::net::SocketAddr;

use axum::Router;

pub(crate) async fn start_server(app: Router) -> Result<SocketAddr, io::Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(address)
}
