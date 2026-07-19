// wsproxy.rs
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use std::io;
use std::collections::HashMap;
use log::{info, warn, error};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use crate::Config;

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Lê headers HTTP até \r\n\r\n
async fn read_http_headers(socket: &mut TcpStream) -> io::Result<(String, HashMap<String, String>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1];
    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 16384 {
            break;
        }
    }
    
    let headers_str = String::from_utf8_lossy(&buf).to_string();
    let mut headers_map = HashMap::new();
    
    for line in headers_str.lines() {
        if let Some((key, value)) = line.split_once(':') {
            headers_map.insert(
                key.trim().to_lowercase(),
                value.trim().to_string()
            );
        }
    }
    
    Ok((headers_str, headers_map))
}

/// Extrai método da primeira linha
fn extract_method(headers: &str) -> Option<String> {
    if let Some(first_line) = headers.lines().next() {
        if let Some(method) = first_line.split_whitespace().next() {
            return Some(method.to_uppercase());
        }
    }
    None
}

/// Calcula Sec-WebSocket-Accept
fn compute_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    let result = hasher.finalize();
    general_purpose::STANDARD.encode(result)
}

/// Verifica se o método é permitido
fn is_method_allowed(method: &str, cfg: &Config) -> bool {
    cfg.allow_methods.iter().any(|m| m.eq_ignore_ascii_case(method))
}

/// Conecta ao destino com suporte VPN
pub async fn connect_with_vpn(target: &str, cfg: &Config) -> io::Result<TcpStream> {
    if cfg.vpn_enabled {
        info!("🔐 Conectando via VPN: {}", cfg.vpn_bind);
        
        if let Some(proxy) = &cfg.vpn_proxy {
            info!("🔗 Via proxy: {}", proxy);
            let proxy_addr = proxy.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidInput, format!("Proxy inválido: {}", e))
            })?;
            return TcpStream::connect(proxy_addr).await;
        }
        
        return TcpStream::connect(target).await;
    }
    
    TcpStream::connect(target).await
}

/// Handle requisição multiprotocolo com suporte a múltiplos métodos HTTP
pub async fn handle_multiprotocol_request(
    mut socket: TcpStream,
    cfg: &Config,
    method: &str,
) -> io::Result<()> {
    info!("📨 Método HTTP detectado: {}", method);
    
    // Verifica se o método é permitido
    if !is_method_allowed(method, cfg) {
        let response = format!(
            "HTTP/1.1 405 Method Not Allowed\r\n\
             Allow: {}\r\n\
             Content-Length: 0\r\n\
             \r\n",
            cfg.allow_methods.join(", ")
        );
        socket.write_all(response.as_bytes()).await?;
        return Ok(());
    }
    
    let (headers_str, headers_map) = read_http_headers(&mut socket).await?;
    
    // Detecção de WebSocket
    let is_websocket = headers_map
        .get("upgrade")
        .map(|v| v.to_lowercase() == "websocket")
        .unwrap_or(false)
        && headers_map.contains_key("sec-websocket-key");
    
    if is_websocket && method == "GET" {
        // Handshake WebSocket
        let client_key = headers_map
            .get("sec-websocket-key")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Chave WebSocket faltando"))?;
        
        let accept_key = compute_accept_key(client_key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\
             X-Protocol-Support: websocket,http11,http2,grpc\r\n\
             X-Allowed-Methods: {}\r\n\
             X-Multistatus: 207 Multi-Status\r\n\
             \r\n",
            accept_key,
            cfg.allow_methods.join(", ")
        );
        
        socket.write_all(response.as_bytes()).await?;
        info!("🔗 WebSocket upgrade OK, conectando ao SSH...");
        
        // Conecta ao destino com VPN
        let remote = connect_with_vpn(&cfg.default_target, cfg).await?;
        info!("✅ Conectado ao {} via {}", cfg.default_target, 
            if cfg.vpn_enabled { "VPN" } else { "rede local" });
        
        let (mut client_reader, mut client_writer) = socket.into_split();
        let (mut remote_reader, mut remote_writer) = remote.into_split();
        
        tokio::try_join!(
            tokio::io::copy(&mut client_reader, &mut remote_writer),
            tokio::io::copy(&mut remote_reader, &mut client_writer)
        )?;
        
        info!("🔚 Conexão encerrada");
        return Ok(());
    }
    
    // Resposta para métodos HTTP
    let content = format!(
        "Método: {}\n\
         Protocolo: HTTP/1.1\n\
         VPN: {}\n\
         Métodos permitidos: {}\n\
         Headers recebidos: {}",
        method,
        if cfg.vpn_enabled { "Ativada" } else { "Desativada" },
        cfg.allow_methods.join(", "),
        headers_map.len()
    );
    
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         X-Protocol-Support: websocket,http11,http2,grpc\r\n\
         X-Allowed-Methods: {}\r\n\
         X-Multistatus: 207 Multi-Status\r\n\
         X-Method: {}\r\n\
         X-VPN-Enabled: {}\r\n\
         \r\n\
         {}",
        content.len(),
        cfg.allow_methods.join(", "),
        method,
        cfg.vpn_enabled,
        content
    );
    
    socket.write_all(response.as_bytes()).await?;
    info!("📨 Resposta enviada para método {}", method);
    Ok(())
}

/// Modo direct/security (resposta direta)
pub async fn handle_direct(mut socket: TcpStream, cfg: &Config) -> io::Result<()> {
    info!("🎭 Modo security: resposta direta com status '{}'", cfg.status);
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         X-Protocol-Support: websocket,http11,http2,grpc\r\n\
         X-Allowed-Methods: {}\r\n\
         X-Multistatus: 207 Multi-Status\r\n\
         X-Mode: security\r\n\
         \r\n\
         {}",
        cfg.status.len(),
        cfg.allow_methods.join(", "),
        cfg.status
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

// Mantém compatibilidade
pub async fn handle_websocket(socket: TcpStream, cfg: &Config) -> io::Result<()> {
    handle_multiprotocol_request(socket, cfg, "GET").await
}
