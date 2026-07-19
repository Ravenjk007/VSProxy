// src/socks5.rs
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::Result;
use log::info;

pub async fn handle_socks5(mut client: TcpStream) -> Result<()> {
    info!("🔐 SOCKS5 connection");
    
    // Handshake SOCKS5
    let mut header = [0u8; 2];
    client.read_exact(&mut header).await?;
    let nmethods = header[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;
    
    // Sem autenticação
    client.write_all(&[0x05, 0x00]).await?;
    
    // Lê requisição
    let mut req = [0u8; 4];
    client.read_exact(&mut req).await?;
    let cmd = req[1];
    let atyp = req[3];
    
    // Resolve endereço baseado no tipo
    let target_addr = match atyp {
        0x01 => { // IPv4
            let mut addr = [0u8; 4];
            client.read_exact(&mut addr).await?;
            let mut port = [0u8; 2];
            client.read_exact(&mut port).await?;
            format!("{}.{}.{}.{}:{}", addr[0], addr[1], addr[2], addr[3], u16::from_be_bytes(port))
        }
        0x03 => { // Domain name
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            client.read_exact(&mut domain).await?;
            let mut port = [0u8; 2];
            client.read_exact(&mut port).await?;
            format!("{}:{}", String::from_utf8_lossy(&domain), u16::from_be_bytes(port))
        }
        0x04 => { // IPv6
            let mut addr = [0u8; 16];
            client.read_exact(&mut addr).await?;
            let mut port = [0u8; 2];
            client.read_exact(&mut port).await?;
            format!("[{}]:{}", 
                (0..8)
                    .map(|i| format!("{:04x}", u16::from_be_bytes([addr[2*i], addr[2*i+1]])))
                    .collect::<Vec<_>>()
                    .join(":"),
                u16::from_be_bytes(port)
            )
        }
        _ => {
            // Endereço não suportado
            client.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("Unsupported address type: {}", atyp);
        }
    };
    
    // Verifica comando (apenas CONNECT)
    if cmd != 0x01 {
        client.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        anyhow::bail!("Unsupported SOCKS command: {}", cmd);
    }
    
    info!("🎯 SOCKS5 target: {}", target_addr);
    
    // Conecta ao destino
    match TcpStream::connect(&target_addr).await {
        Ok(remote) => {
            // Resposta de sucesso
            client.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            
            info!("✅ Tunnel established to {}", target_addr);
            
            // Bidirecional copy
            let (mut client_reader, mut client_writer) = client.into_split();
            let (mut remote_reader, mut remote_writer) = remote.into_split();
            
            tokio::try_join!(
                tokio::io::copy(&mut client_reader, &mut remote_writer),
                tokio::io::copy(&mut remote_reader, &mut client_writer)
            )?;
            
            info!("🔌 Tunnel closed for {}", target_addr);
            Ok(())
        }
        Err(e) => {
            info!("❌ Connection failed to {}: {}", target_addr, e);
            client.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("Connection failed: {}", e);
        }
    }
}
