use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;
use log::{info, error, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use std::net::{Ipv4Addr, Ipv6Addr};

// ============================================
// ESTATÍSTICAS (MULTISTATUS)
// ============================================
pub struct Stats {
    pub active_connections: AtomicUsize,
    pub total_websocket: AtomicUsize,
    pub total_socks5: AtomicUsize,
    pub total_security: AtomicUsize,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_websocket: AtomicUsize::new(0),
            total_socks5: AtomicUsize::new(0),
            total_security: AtomicUsize::new(0),
        }
    }
}

// ============================================
// CONSTANTES E AUXILIARES
// ============================================
const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const VALID_TOKENS: &[&str] = &["meu-token-seguro-123", "admin-uuid-456"];

pub async fn read_http_headers(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 1];
    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" { break; }
        if buf.len() > 8192 { break; }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

pub fn extract_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    let name_lower = name.to_lowercase();
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_lowercase() == name_lower { return Some(v.trim()); }
        }
    }
    None
}

// ============================================
// PROTOCOLO WEBSOCKET
// ============================================
async fn handle_websocket(mut socket: TcpStream, headers: String) -> Result<()> {
    let client_key = match extract_header(&headers, "Sec-WebSocket-Key") {
        Some(k) => k.to_string(),
        None => {
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            socket.write_all(response.as_bytes()).await?;
            anyhow::bail!("Missing Sec-WebSocket-Key");
        }
    };

    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    let accept_key = general_purpose::STANDARD.encode(hasher.finalize());

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key
    );
    socket.write_all(response.as_bytes()).await?;
    forward_to_ssh(socket).await
}

// ============================================
// PROTOCOLO SOCKS5
// ============================================
async fn handle_socks5(mut socket: TcpStream) -> Result<()> {
    let mut buf = [0u8; 2];
    socket.read_exact(&mut buf).await?;
    let nmethods = buf[0];
    let mut methods = vec![0u8; nmethods as usize];
    socket.read_exact(&mut methods).await?;
    socket.write_all(&[0x05, 0x00]).await?;

    let mut header = [0u8; 4];
    socket.read_exact(&mut header).await?;
    let target_addr = match header[3] {
        0x01 => {
            let mut addr = [0u8; 4];
            socket.read_exact(&mut addr).await?;
            format!("{}", Ipv4Addr::from(addr))
        }
        0x03 => {
            let len = socket.read_u8().await?;
            let mut domain = vec![0u8; len as usize];
            socket.read_exact(&mut domain).await?;
            String::from_utf8_lossy(&domain).to_string()
        }
        _ => anyhow::bail!("Unsupported address type"),
    };
    let port = socket.read_u16().await?;
    let target = format!("{}:{}", target_addr, port);
    
    match TcpStream::connect(&target).await {
        Ok(remote) => {
            socket.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            proxy_bridge(socket, remote).await
        }
        Err(_) => {
            socket.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5 connect failed")
        }
    }
}

// ============================================
// PROTOCOLO SECURITY
// ============================================
async fn handle_security(mut socket: TcpStream, headers: String) -> Result<()> {
    let token = extract_header(&headers, "X-Proxy-Token")
        .or_else(|| extract_header(&headers, "Authorization"))
        .map(|t| t.trim_start_matches("Bearer ").trim());

    if let Some(t) = token {
        if VALID_TOKENS.contains(&t) {
            socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
            return forward_to_ssh(socket).await;
        }
    }
    socket.write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n").await?;
    anyhow::bail!("Unauthorized")
}

// ============================================
// CORE BRIDGE
// ============================================
async fn forward_to_ssh(socket: TcpStream) -> Result<()> {
    let remote = TcpStream::connect("127.0.0.1:22").await?;
    proxy_bridge(socket, remote).await
}

async fn proxy_bridge(socket: TcpStream, remote: TcpStream) -> Result<()> {
    let (mut c_r, mut c_w) = socket.into_split();
    let (mut r_r, mut r_w) = remote.into_split();
    tokio::try_join!(tokio::io::copy(&mut c_r, &mut r_w), tokio::io::copy(&mut r_r, &mut c_w))?;
    Ok(())
}

// ============================================
// MAIN DISPATCHER
// ============================================
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("🚀 VSProxy Unified Server listening on 8080");
    let stats = Arc::new(Stats::new());

    loop {
        let (socket, _) = listener.accept().await?;
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            stats.active_connections.fetch_add(1, Ordering::SeqCst);
            let _ = handle_connection(socket, stats.clone()).await;
            stats.active_connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

async fn handle_connection(mut socket: TcpStream, stats: Arc<Stats>) -> Result<()> {
    let mut first_byte = [0u8; 1];
    socket.read_exact(&mut first_byte).await?;

    match first_byte[0] {
        0x05 => {
            stats.total_socks5.fetch_add(1, Ordering::SeqCst);
            handle_socks5(socket).await
        }
        b'G' | b'P' | b'C' | b'H' | b'D' | b'O' | b'T' => {
            let mut headers = String::from_utf8_lossy(&[first_byte[0]]).to_string();
            headers.push_str(&read_http_headers(&mut socket).await?);
            if headers.contains("Upgrade: websocket") || extract_header(&headers, "Sec-WebSocket-Key").is_some() {
                stats.total_websocket.fetch_add(1, Ordering::SeqCst);
                handle_websocket(socket, headers).await
            } else if extract_header(&headers, "X-Proxy-Token").is_some() || extract_header(&headers, "Authorization").is_some() {
                stats.total_security.fetch_add(1, Ordering::SeqCst);
                handle_security(socket, headers).await
            } else {
                anyhow::bail!("Unsupported HTTP")
            }
        }
        _ => anyhow::bail!("Unknown protocol"),
    }
}
