// wsproxy.rs
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use std::io;
use log::{info, warn, error};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use crate::Config;

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const HTTP_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";

/// Lê headers HTTP até \r\n\r\n
async fn read_http_headers(socket: &mut TcpStream) -> io::Result<String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1];
    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 8192 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Extrai valor de header
fn extract_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    let name_lower = name.to_lowercase();
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_lowercase() == name_lower {
                return Some(v.trim());
            }
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

/// Tipos de requisição
#[derive(Debug, PartialEq)]
pub enum RequestType {
    WebSocket,
    Http11,
    Http2,
    Grpc,
    Unknown,
}

/// Analisa tipo de requisição
pub fn analyze_request(headers: &str) -> RequestType {
    // WebSocket
    if let Some(upgrade) = extract_header(headers, "Upgrade") {
        if upgrade.to_lowercase() == "websocket" && extract_header(headers, "Sec-WebSocket-Key").is_some() {
            return RequestType::WebSocket;
        }
    }
    
    // gRPC
    if extract_header(headers, "Content-Type").map_or(false, |ct| ct.contains("application/grpc")) {
        return RequestType::Grpc;
    }
    
    // HTTP/2
    if extract_header(headers, "HTTP-Version").map_or(false, |v| v.contains("2.")) {
        return RequestType::Http2;
    }
    
    // HTTP/1.1 padrão
    if headers.starts_with("GET") || headers.starts_with("POST") || 
       headers.starts_with("PUT") || headers.starts_with("DELETE") {
        return RequestType::Http11;
    }
    
    RequestType::Unknown
}

/// Conecta ao destino com suporte VPN
pub async fn connect_with_vpn(target: &str, cfg: &Config) -> io::Result<TcpStream> {
    if cfg.vpn_enabled {
        info!("🔐 Conectando via VPN: {}", cfg.vpn_bind);
        
        // Se tiver proxy, usa ele
        if let Some(proxy) = &cfg.vpn_proxy {
            info!("🔗 Via proxy: {}", proxy);
            let proxy_addr = proxy.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidInput, format!("Proxy inválido: {}", e))
            })?;
            return TcpStream::connect(proxy_addr).await;
        }
        
        // Conexão normal (bind seria feito no socket em implementação real)
        return TcpStream::connect(target).await;
    }
    
    TcpStream::connect(target).await
}

/// Handle WebSocket com suporte multiprotocolo
pub async fn handle_multiprotocol_websocket(mut socket: TcpStream, cfg: &Config) -> io::Result<()> {
    info!("🌐 Processando requisição multi-protocolo...");
    
    let headers = read_http_headers(&mut socket).await?;
    let request_type = analyze_request(&headers);
    info!("📡 Tipo: {:?}", request_type);
    
    // Resposta com suporte multiprotocolo
    if request_type == RequestType::WebSocket {
        let client_key = extract_header(&headers, "Sec-WebSocket-Key")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Chave WebSocket faltando"))?;
        
        let accept_key = compute_accept_key(client_key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\
             X-Protocol-Support: websocket,http11,http2,grpc\r\n\
             X-Multistatus: 207 Multi-Status\r\n\
             \r\n",
            accept_key
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
        Ok(())
    } else {
        // Resposta para outros protocolos
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 60\r\n\
             X-Protocol-Support: websocket,http11,http2,grpc\r\n\
             X-Multistatus: 207 Multi-Status\r\n\
             \r\n\
             Protocolo detectado: {:?}. Use WebSocket para conexão SSH.",
            request_type
        );
        
        socket.write_all(response.as_bytes()).await?;
        info!("📨 Resposta multiprotocolo enviada");
        Ok(())
    }
}

/// Handle HTTP com suporte multiprotocolo
pub async fn handle_multiprotocol_http(mut socket: TcpStream, cfg: &Config) -> io::Result<()> {
    let headers = read_http_headers(&mut socket).await?;
    let request_type = analyze_request(&headers);
    
    info!("📡 Requisição HTTP: {:?}", request_type);
    
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json\r\n\
         Content-Length: 100\r\n\
         X-Protocol-Support: websocket,http11,http2,grpc\r\n\
         X-Multistatus: 207 Multi-Status\r\n\
         \r\n\
         {{ \"protocol\": \"{:?}\", \"status\": \"OK\", \"vpn\": {} }}",
        request_type, cfg.vpn_enabled
    );
    
    socket.write_all(response.as_bytes()).await?;
    info!("📨 Resposta JSON enviada");
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
         X-Multistatus: 207 Multi-Status\r\n\
         \r\n\
         {}",
        cfg.status.len(),
        cfg.status
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

// Mantém a função original para compatibilidade
pub async fn handle_websocket(socket: TcpStream, cfg: &Config) -> io::Result<()> {
    handle_multiprotocol_websocket(socket, cfg).await
}
