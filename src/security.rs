use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use anyhow::Result;
use log::{info, warn};
use crate::protocols::websocket::extract_header;

// In a real scenario, these would be loaded from a config file
const VALID_TOKENS: &[&str] = &["meu-token-seguro-123", "admin-uuid-456"];

pub async fn handle_security(mut socket: TcpStream, headers: String) -> Result<()> {
    info!("🔐 Verifying SECURITY protocol...");

    let auth_token = extract_header(&headers, "X-Proxy-Token")
        .or_else(|| extract_header(&headers, "Authorization"));

    let is_authorized = match auth_token {
        Some(token) => {
            let token = token.trim_start_matches("Bearer ").trim();
            VALID_TOKENS.contains(&token)
        }
        None => false,
    };

    if !is_authorized {
        warn!("🚫 Unauthorized SECURITY access attempt");
        let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
        socket.write_all(response.as_bytes()).await?;
        anyhow::bail!("Unauthorized token");
    }

    info!("✅ SECURITY Authorized! Forwarding...");
    
    // Once authorized, we can treat it as a transparent tunnel or a specific protocol
    // For this implementation, let's forward to SSH as a default target
    let target = "127.0.0.1:22";
    match TcpStream::connect(target).await {
        Ok(remote) => {
            let response = "HTTP/1.1 200 OK\r\n\r\n";
            socket.write_all(response.as_bytes()).await?;
            
            let (mut client_reader, mut client_writer) = socket.into_split();
            let (mut remote_reader, mut remote_writer) = remote.into_split();

            tokio::try_join!(
                tokio::io::copy(&mut client_reader, &mut remote_writer),
                tokio::io::copy(&mut remote_reader, &mut client_writer)
            )?;
            Ok(())
        }
        Err(e) => {
            warn!("❌ SECURITY failed to connect to target: {}", e);
            anyhow::bail!("Target connection failed")
        }
    }
}
