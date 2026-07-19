use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use anyhow::Result;
use log::info;

pub async fn handle_tcp(mut socket: TcpStream) -> Result<()> {
    info!("📦 TCP Fallback");
    socket.write_all(b"TCP OK\n").await?;
    Ok(())
}
