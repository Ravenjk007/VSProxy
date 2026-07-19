mod protocols;

use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncReadExt;
use anyhow::Result;
use log::{info, error, warn};
use std::sync::Arc;

use crate::protocols::websocket::{handle_websocket, read_http_headers, extract_header};
use crate::protocols::socks5::handle_socks5;
use crate::protocols::security::handle_security;
use crate::protocols::multistatus::Stats;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    
    let addr = "0.0.0.0:8080";
    let listener = TcpListener::bind(addr).await?;
    info!("🚀 Multi-Proxy Server listening on {}", addr);

    let stats = Arc::new(Stats::new());

    loop {
        let (socket, client_addr) = listener.accept().await?;
        let stats = Arc::clone(&stats);
        
        tokio::spawn(async move {
            stats.add_connection();
            if let Err(e) = handle_connection(socket, stats.clone()).await {
                error!("Error handling connection from {}: {}", client_addr, e);
            }
            stats.remove_connection();
        });
    }
}

async fn handle_connection(mut socket: TcpStream, stats: Arc<Stats>) -> Result<()> {
    // Peek at the first byte to determine the protocol
    let mut first_byte = [0u8; 1];
    socket.read_exact(&mut first_byte).await?;

    match first_byte[0] {
        0x05 => {
            info!("⚡ Detected SOCKS5 protocol");
            stats.inc_socks5();
            handle_socks5(socket).await
        }
        b'G' | b'P' | b'C' | b'H' | b'D' | b'O' | b'T' => { // HTTP Methods (GET, POST, CONNECT, etc)
            let mut headers = String::from_utf8_lossy(&[first_byte[0]]).to_string();
            headers.push_str(&read_http_headers(&mut socket).await?);
            
            if headers.contains("Upgrade: websocket") || extract_header(&headers, "Sec-WebSocket-Key").is_some() {
                info!("⚡ Detected WebSocket protocol");
                stats.inc_websocket();
                handle_websocket(socket, headers).await
            } else if extract_header(&headers, "X-Proxy-Token").is_some() || extract_header(&headers, "Authorization").is_some() {
                info!("⚡ Detected SECURITY protocol");
                stats.inc_security();
                handle_security(socket, headers).await
            } else {
                info!("⚡ Detected standard HTTP/CONNECT protocol");
                anyhow::bail!("Standard HTTP not implemented yet")
            }
        }
        _ => {
            warn!("❓ Unknown protocol first byte: 0x{:02X}", first_byte[0]);
            anyhow::bail!("Unknown protocol")
        }
    }
}
