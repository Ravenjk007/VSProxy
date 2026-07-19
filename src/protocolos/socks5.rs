use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::Result;
use log::{info, warn};
use std::net::{Ipv4Addr, Ipv6Addr};

pub async fn handle_socks5(mut socket: TcpStream) -> Result<()> {
    // SOCKS5 Handshake: [VERSION, NMETHODS, METHODS...]
    // We already read the first byte (0x05) in the dispatcher, but let's assume we are at the start of the handshake or passed it.
    // If the dispatcher passed the socket after reading 0x05, we need to handle that.
    
    let mut buf = [0u8; 2];
    socket.read_exact(&mut buf).await?;
    
    let nmethods = buf[0];
    let mut methods = vec![0u8; nmethods as usize];
    socket.read_exact(&mut methods).await?;
    
    // Support only 'No Authentication' for now (0x00)
    if !methods.contains(&0x00) {
        socket.write_all(&[0x05, 0xFF]).await?;
        anyhow::bail!("No supported SOCKS5 auth methods");
    }
    
    socket.write_all(&[0x05, 0x00]).await?; // No auth selected
    
    // Request: [VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT]
    let mut header = [0u8; 4];
    socket.read_exact(&mut header).await?;
    
    if header[1] != 0x01 { // Only CONNECT supported
        socket.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        anyhow::bail!("SOCKS5 command not supported");
    }
    
    let target_addr = match header[3] {
        0x01 => { // IPv4
            let mut addr = [0u8; 4];
            socket.read_exact(&mut addr).await?;
            format!("{}", Ipv4Addr::from(addr))
        }
        0x03 => { // Domain name
            let len = socket.read_u8().await?;
            let mut domain = vec![0u8; len as usize];
            socket.read_exact(&mut domain).await?;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => { // IPv6
            let mut addr = [0u8; 16];
            socket.read_exact(&mut addr).await?;
            format!("{}", Ipv6Addr::from(addr))
        }
        _ => {
            socket.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5 address type not supported");
        }
    };
    
    let port = socket.read_u16().await?;
    let target = format!("{}:{}", target_addr, port);
    
    info!("🔌 SOCKS5 Connecting to {}", target);
    
    match TcpStream::connect(&target).await {
        Ok(remote) => {
            // Success response
            socket.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            
            let (mut client_reader, mut client_writer) = socket.into_split();
            let (mut remote_reader, mut remote_writer) = remote.into_split();

            tokio::try_join!(
                tokio::io::copy(&mut client_reader, &mut remote_writer),
                tokio::io::copy(&mut remote_reader, &mut client_writer)
            )?;
            Ok(())
        }
        Err(e) => {
            warn!("❌ SOCKS5 failed to connect to {}: {}", target, e);
            socket.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5 target connection failed")
        }
    }
}
