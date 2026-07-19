use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{Result, anyhow};
use log::{info, error, warn, debug};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use tokio::time::timeout;
use tokio::sync::Semaphore;

// ============================================
// CONFIGURAÇÕES
// ============================================
#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub ssh_addr: String,
    pub connection_timeout: Duration,
    pub max_connections: usize,
    pub buffer_size: usize,
    pub valid_tokens: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".to_string(),
            ssh_addr: "127.0.0.1:22".to_string(),
            connection_timeout: Duration::from_secs(30),
            max_connections: 1000,
            buffer_size: 8192,
            valid_tokens: vec![
                "meu-token-seguro-123".to_string(),
                "admin-uuid-456".to_string()
            ],
        }
    }
}

// ============================================
// ESTATÍSTICAS MELHORADAS
// ============================================
pub struct Stats {
    pub active_connections: AtomicUsize,
    pub total_websocket: AtomicUsize,
    pub total_socks5: AtomicUsize,
    pub total_security: AtomicUsize,
    pub total_errors: AtomicUsize,
    pub total_bytes_transferred: AtomicUsize,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_websocket: AtomicUsize::new(0),
            total_socks5: AtomicUsize::new(0),
            total_security: AtomicUsize::new(0),
            total_errors: AtomicUsize::new(0),
            total_bytes_transferred: AtomicUsize::new(0),
        }
    }

    pub fn increment_bytes(&self, bytes: usize) {
        self.total_bytes_transferred.fetch_add(bytes, Ordering::SeqCst);
    }
}

// ============================================
// READER COM TIMEOUT
// ============================================
async fn read_http_headers_with_timeout(
    socket: &mut TcpStream,
    timeout_duration: Duration,
) -> std::io::Result<String> {
    timeout(timeout_duration, read_http_headers(socket))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout reading headers"))?
}

async fn read_http_headers(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 1];
    let mut total_read = 0;
    
    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);
        total_read += 1;
        
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if total_read > 8192 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Headers too large",
            ));
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

// ============================================
// HEADER PARSER MELHORADO
// ============================================
pub struct HttpHeaders {
    pub raw: String,
    parsed: std::collections::HashMap<String, String>,
}

impl HttpHeaders {
    pub fn new(raw: String) -> Self {
        let mut parsed = std::collections::HashMap::new();
        for line in raw.lines() {
            if let Some((k, v)) = line.split_once(':') {
                parsed.insert(
                    k.trim().to_lowercase(),
                    v.trim().to_string(),
                );
            }
        }
        Self { raw, parsed }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.parsed.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    pub fn is_websocket(&self) -> bool {
        self.raw.contains("Upgrade: websocket") 
            || self.get("Sec-WebSocket-Key").is_some()
    }

    pub fn has_auth(&self) -> bool {
        self.get("X-Proxy-Token").is_some() 
            || self.get("Authorization").is_some()
    }
}

// ============================================
// PROTOCOLO WEBSOCKET
// ============================================
async fn handle_websocket(
    mut socket: TcpStream,
    headers: HttpHeaders,
    config: &Config,
) -> Result<()> {
    let client_key = headers.get("Sec-WebSocket-Key")
        .ok_or_else(|| anyhow!("Missing Sec-WebSocket-Key"))?;

    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept_key = general_purpose::STANDARD.encode(hasher.finalize());

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key
    );
    
    timeout(config.connection_timeout, socket.write_all(response.as_bytes()))
        .await
        .map_err(|_| anyhow!("Timeout writing websocket response"))??;
    
    forward_to_ssh(socket, config).await
}

// ============================================
// PROTOCOLO SOCKS5
// ============================================
async fn handle_socks5(
    mut socket: TcpStream,
    config: &Config,
) -> Result<()> {
    // Handshake
    let mut buf = [0u8; 2];
    timeout(config.connection_timeout, socket.read_exact(&mut buf)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 handshake"))??;
    
    let nmethods = buf[1];
    let mut methods = vec![0u8; nmethods as usize];
    timeout(config.connection_timeout, socket.read_exact(&mut methods)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 methods"))??;
    
    socket.write_all(&[0x05, 0x00]).await?;

    // Request
    let mut header = [0u8; 4];
    timeout(config.connection_timeout, socket.read_exact(&mut header)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 request"))??;
    
    let target_addr = match header[3] {
        0x01 => {
            let mut addr = [0u8; 4];
            timeout(config.connection_timeout, socket.read_exact(&mut addr)).await
                .map_err(|_| anyhow!("Timeout reading IPv4 address"))??;
            format!("{}", Ipv4Addr::from(addr))
        }
        0x03 => {
            let len = timeout(config.connection_timeout, socket.read_u8()).await
                .map_err(|_| anyhow!("Timeout reading domain length"))??;
            let mut domain = vec![0u8; len as usize];
            timeout(config.connection_timeout, socket.read_exact(&mut domain)).await
                .map_err(|_| anyhow!("Timeout reading domain"))??;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => {
            let mut addr = [0u8; 16];
            timeout(config.connection_timeout, socket.read_exact(&mut addr)).await
                .map_err(|_| anyhow!("Timeout reading IPv6 address"))??;
            format!("{}", Ipv6Addr::from(addr))
        }
        _ => anyhow::bail!("Unsupported address type: {}", header[3]),
    };
    
    let port = timeout(config.connection_timeout, socket.read_u16()).await
        .map_err(|_| anyhow!("Timeout reading port"))??;
    let target = format!("{}:{}", target_addr, port);
    debug!("SOCKS5 target: {}", target);
    
    match TcpStream::connect(&target).await {
        Ok(remote) => {
            // Response: success
            let response = match header[3] {
                0x01 => vec![0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0],
                0x04 => vec![0x05, 0x00, 0x00, 0x04, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0],
                _ => vec![0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0],
            };
            
            timeout(config.connection_timeout, socket.write_all(&response)).await
                .map_err(|_| anyhow!("Timeout writing SOCKS5 response"))??;
            
            proxy_bridge(socket, remote, config).await
        }
        Err(e) => {
            error!("SOCKS5 connect failed: {}", e);
            socket.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5 connect failed: {}", e)
        }
    }
}

// ============================================
// PROTOCOLO SECURITY
// ============================================
async fn handle_security(
    mut socket: TcpStream,
    headers: HttpHeaders,
    config: &Config,
) -> Result<()> {
    let token = headers.get("X-Proxy-Token")
        .or_else(|| headers.get("Authorization"))
        .map(|t| t.trim_start_matches("Bearer ").trim());

    if let Some(t) = token {
        if config.valid_tokens.contains(&t.to_string()) {
            socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
            return forward_to_ssh(socket, config).await;
        }
    }
    
    warn!("Unauthorized access attempt");
    socket.write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n").await?;
    anyhow::bail!("Unauthorized")
}

// ============================================
// CORE BRIDGE MELHORADO
// ============================================
async fn forward_to_ssh(socket: TcpStream, config: &Config) -> Result<()> {
    let remote = TcpStream::connect(&config.ssh_addr).await?;
    proxy_bridge(socket, remote, config).await
}

async fn proxy_bridge(
    client: TcpStream,
    remote: TcpStream,
    config: &Config,
) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    
    // Aplicar timeout e buffer otimizado
    let client_to_remote = async {
        let mut buffer = vec![0u8; config.buffer_size];
        loop {
            match timeout(
                config.connection_timeout,
                client_read.read(&mut buffer)
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    remote_write.write_all(&buffer[..n]).await?;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => break,
            }
        }
        Ok::<_, std::io::Error>(())
    };

    let remote_to_client = async {
        let mut buffer = vec![0u8; config.buffer_size];
        loop {
            match timeout(
                config.connection_timeout,
                remote_read.read(&mut buffer)
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    client_write.write_all(&buffer[..n]).await?;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => break,
            }
        }
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = client_to_remote => result?,
        result = remote_to_client => result?,
    }
    
    Ok(())
}

// ============================================
// MAIN DISPATCHER MELHORADO
// ============================================
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().default_filter_or("info")
    );
    
    let config = Config::default();
    let listener = TcpListener::bind(&config.bind_addr).await?;
    info!("🚀 VSProxy Unified Server running on {}", config.bind_addr);
    info!("   SSH target: {}", config.ssh_addr);
    info!("   Max connections: {}", config.max_connections);
    info!("   Timeout: {}s", config.connection_timeout.as_secs());
    
    let stats = Arc::new(Stats::new());
    let semaphore = Arc::new(Semaphore::new(config.max_connections));
    
    loop {
        let (socket, addr) = listener.accept().await?;
        let stats = Arc::clone(&stats);
        let config = config.clone();
        let semaphore = Arc::clone(&semaphore);
        
        tokio::spawn(async move {
            let _permit = semaphore.acquire().await;
            stats.active_connections.fetch_add(1, Ordering::SeqCst);
            
            if let Err(e) = handle_connection(socket, stats.clone(), &config).await {
                error!("Connection error from {}: {}", addr, e);
                stats.total_errors.fetch_add(1, Ordering::SeqCst);
            }
            
            stats.active_connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    stats: Arc<Stats>,
    config: &Config,
) -> Result<()> {
    let mut first_byte = [0u8; 1];
    timeout(config.connection_timeout, socket.read_exact(&mut first_byte)).await
        .map_err(|_| anyhow!("Timeout reading first byte"))??;

    match first_byte[0] {
        0x05 => {
            stats.total_socks5.fetch_add(1, Ordering::SeqCst);
            info!("SOCKS5 connection detected");
            handle_socks5(socket, config).await
        }
        b'G' | b'P' | b'C' | b'H' | b'D' | b'O' | b'T' => {
            let mut headers_str = String::from_utf8_lossy(&[first_byte[0]]).to_string();
            headers_str.push_str(&read_http_headers_with_timeout(&mut socket, config.connection_timeout).await?);
            
            let headers = HttpHeaders::new(headers_str);
            
            if headers.is_websocket() {
                stats.total_websocket.fetch_add(1, Ordering::SeqCst);
                info!("WebSocket connection detected");
                handle_websocket(socket, headers, config).await
            } else if headers.has_auth() {
                stats.total_security.fetch_add(1, Ordering::SeqCst);
                info!("Security connection detected");
                handle_security(socket, headers, config).await
            } else {
                anyhow::bail!("Unsupported HTTP request")
            }
        }
        _ => anyhow::bail!("Unknown protocol: 0x{:02x}", first_byte[0]),
    }
}

// ============================================
// MÓDULO DE HEALTH CHECK
// ============================================
pub async fn health_check(stats: Arc<Stats>) -> String {
    format!(
        "Status: OK\n\
         Active connections: {}\n\
         Total WebSocket: {}\n\
         Total SOCKS5: {}\n\
         Total Security: {}\n\
         Total Errors: {}\n\
         Total Bytes: {} MB",
        stats.active_connections.load(Ordering::SeqCst),
        stats.total_websocket.load(Ordering::SeqCst),
        stats.total_socks5.load(Ordering::SeqCst),
        stats.total_security.load(Ordering::SeqCst),
        stats.total_errors.load(Ordering::SeqCst),
        stats.total_bytes_transferred.load(Ordering::SeqCst) / 1_000_000
    )
}
