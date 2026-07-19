use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::{Result, bail, anyhow};
use log::{info, warn, error};
use std::collections::HashMap;
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Configuração para TLS com suporte VPN
#[derive(Clone)]
pub struct TlsConfig {
    pub default_target: String,
    pub vpn_enabled: bool,
    pub vpn_bind: String,
    pub vpn_proxy: Option<String>,
    pub allow_methods: Vec<String>,
    pub status: String,
}

/// Lê o TLS ClientHello completo (baseado no tamanho declarado no record header)
async fn read_client_hello(socket: &mut TcpStream) -> Result<Vec<u8>> {
    // TLS record header: [type(1)][version(2)][length(2)]
    let mut record_header = [0u8; 5];
    socket.read_exact(&mut record_header).await?;
    
    if record_header[0] != 0x16 {
        bail!("Not a TLS handshake record");
    }
    
    let record_len = u16::from_be_bytes([record_header[3], record_header[4]]) as usize;
    let mut body = vec![0u8; record_len];
    socket.read_exact(&mut body).await?;
    
    let mut full = record_header.to_vec();
    full.extend_from_slice(&body);
    Ok(full)
}

/// Extrai o hostname da extensão SNI dentro de um TLS ClientHello (buffer completo, com record header)
pub fn extract_sni(data: &[u8]) -> Option<String> {
    // Pula: record header(5) + handshake header(4) + version(2) + random(32)
    let mut pos = 5 + 4 + 2 + 32;
    if pos >= data.len() {
        return None;
    }
    
    // session id
    let session_id_len = *data.get(pos)? as usize;
    pos += 1 + session_id_len;
    if pos + 2 > data.len() {
        return None;
    }
    
    // cipher suites
    let cipher_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2 + cipher_len;
    if pos >= data.len() {
        return None;
    }
    
    // compression methods
    let comp_len = *data.get(pos)? as usize;
    pos += 1 + comp_len;
    if pos + 2 > data.len() {
        return None;
    }
    
    // extensions total length
    let ext_total_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    let ext_end = (pos + ext_total_len).min(data.len());
    
    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let ext_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > data.len() {
            break;
        }
        if ext_type == 0x0000 { // server_name extension
            // [list_len(2)][type(1)][name_len(2)][name...]
            let ext_data = &data[pos..pos + ext_len];
            if ext_data.len() >= 5 {
                let name_len = u16::from_be_bytes([ext_data[3], ext_data[4]]) as usize;
                if ext_data.len() >= 5 + name_len {
                    let name = &ext_data[5..5 + name_len];
                    return Some(String::from_utf8_lossy(name).to_string());
                }
            }
        }
        pos += ext_len;
    }
    None
}

/// Lê headers HTTP de uma conexão TLS
async fn read_http_headers(socket: &mut (impl AsyncReadExt + Unpin)) -> std::io::Result<(String, HashMap<String, String>)> {
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
fn is_method_allowed(method: &str, cfg: &TlsConfig) -> bool {
    cfg.allow_methods.iter().any(|m| m.eq_ignore_ascii_case(method))
}

/// Conecta ao destino com suporte VPN
async fn connect_with_vpn(target: &str, cfg: &TlsConfig) -> std::io::Result<TcpStream> {
    if cfg.vpn_enabled {
        info!("🔐 Conectando via VPN: {}", cfg.vpn_bind);
        
        if let Some(proxy) = &cfg.vpn_proxy {
            info!("🔗 Via proxy: {}", proxy);
            let proxy_addr = proxy.parse().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("Proxy inválido: {}", e))
            })?;
            return TcpStream::connect(proxy_addr).await;
        }
        
        return TcpStream::connect(target).await;
    }
    
    TcpStream::connect(target).await
}

/// Detecta se a conexão é HTTP/HTTPS ou TLS puro
async fn detect_protocol(mut socket: &mut TcpStream) -> Result<bool> {
    let mut peek_buf = [0u8; 5];
    let n = socket.peek(&mut peek_buf).await?;
    
    if n >= 3 {
        // Verifica se começa com método HTTP
        let methods = [
            b"GET", b"POST", b"PUT", b"DELETE", b"HEAD", 
            b"PATCH", b"OPTIONS", b"TRACE", b"CONNECT",
            b"ACL", b"MOVE", b"COPY", b"LINK", b"UNLINK",
            b"PURGE", b"LOCK", b"UNLOCK", b"PROPFIND", b"VIEW"
        ];
        
        for method in &methods {
            if n >= method.len() && &peek_buf[0..method.len()] == *method {
                return Ok(true);
            }
        }
    }
    
    // Verifica se é TLS handshake
    if n >= 1 && peek_buf[0] == 0x16 {
        return Ok(false);
    }
    
    // Assume TLS por padrão
    Ok(false)
}

pub async fn handle_tls(mut client: TcpStream, cfg: Option<TlsConfig>) -> Result<()> {
    let cfg = cfg.unwrap_or_else(|| TlsConfig {
        default_target: "127.0.0.1:22".to_string(),
        vpn_enabled: false,
        vpn_bind: "0.0.0.0:0".to_string(),
        vpn_proxy: None,
        allow_methods: vec![
            "GET".to_string(),
            "POST".to_string(),
            "PUT".to_string(),
            "DELETE".to_string(),
            "CONNECT".to_string(),
            "HEAD".to_string(),
            "PATCH".to_string(),
            "OPTIONS".to_string(),
            "TRACE".to_string(),
            "ACL".to_string(),
            "MOVE".to_string(),
            "COPY".to_string(),
            "LINK".to_string(),
            "UNLINK".to_string(),
            "PURGE".to_string(),
            "LOCK".to_string(),
            "UNLOCK".to_string(),
            "PROPFIND".to_string(),
            "VIEW".to_string(),
        ],
        status: "@VSProxy".to_string(),
    });
    
    // Detecta se é HTTP ou TLS puro
    let is_http = detect_protocol(&mut client).await?;
    
    if is_http {
        info!("📨 Conexão TLS detectada como HTTP/HTTPS");
        return handle_http_over_tls(client, cfg).await;
    }
    
    // TLS puro (SNI passthrough)
    info!("🔒 TLS handshake, lendo ClientHello...");
    let hello = read_client_hello(&mut client).await?;
    let sni = extract_sni(&hello);
    
    let target = match &sni {
        Some(host) => {
            info!("🔒 TLS SNI -> {}", host);
            format!("{}:443", host)
        }
        None => {
            warn!("⚠️ Sem SNI no ClientHello, usando destino padrão");
            cfg.default_target.clone()
        }
    };
    
    info!("🎯 Conectando ao destino: {}", target);
    
    // Conecta ao destino com VPN
    let remote = match connect_with_vpn(&target, &cfg).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("❌ Falha ao conectar em {}: {}", target, e);
            bail!("Falha ao conectar em {}: {}", target, e);
        }
    };
    
    info!("✅ Conectado ao {} via {}", target, 
        if cfg.vpn_enabled { "VPN" } else { "rede local" });
    
    // Reenviar o ClientHello que já consumimos
    let mut remote = remote;
    remote.write_all(&hello).await?;
    
    let (mut client_reader, mut client_writer) = client.into_split();
    let (mut remote_reader, mut remote_writer) = remote.into_split();
    
    tokio::try_join!(
        tokio::io::copy(&mut client_reader, &mut remote_writer),
        tokio::io::copy(&mut remote_reader, &mut client_writer)
    )?;
    
    info!("🔚 Conexão TLS encerrada");
    Ok(())
}

/// Handler para HTTP sobre TLS (requisições HTTPS/HTTP com SNI)
async fn handle_http_over_tls(mut client: TcpStream, cfg: TlsConfig) -> Result<()> {
    info!("📨 Processando HTTP sobre TLS...");
    
    let (headers_str, headers_map) = match read_http_headers(&mut client).await {
        Ok(data) => data,
        Err(e) => {
            error!("❌ Erro ao ler headers HTTP: {}", e);
            bail!("Falha ao ler headers HTTP: {}", e);
        }
    };
    
    let method = extract_method(&headers_str).unwrap_or_else(|| "UNKNOWN".to_string());
    let host = headers_map.get("host").cloned().unwrap_or_else(|| "localhost".to_string());
    let path = headers_str.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    
    info!("📨 Método: {}, Host: {}, Path: {}", method, host, path);
    
    // Verifica se o método é permitido
    if !is_method_allowed(&method, &cfg) {
        let response = format!(
            "HTTP/1.1 405 Method Not Allowed\r\n\
             Allow: {}\r\n\
             Content-Length: 0\r\n\
             X-Protocol-Support: websocket,http11,http2,grpc\r\n\
             X-Allowed-Methods: {}\r\n\
             X-Multistatus: 207 Multi-Status\r\n\
             \r\n",
            cfg.allow_methods.join(", "),
            cfg.allow_methods.join(", ")
        );
        client.write_all(response.as_bytes()).await?;
        return Ok(());
    }
    
    // Detecção de WebSocket sobre TLS (WSS)
    let is_websocket = headers_map
        .get("upgrade")
        .map(|v| v.to_lowercase() == "websocket")
        .unwrap_or(false)
        && headers_map.contains_key("sec-websocket-key")
        && method == "GET";
    
    if is_websocket {
        // Handshake WebSocket sobre TLS (WSS)
        let client_key = headers_map
            .get("sec-websocket-key")
            .ok_or_else(|| anyhow!("Chave WebSocket faltando"))?;
        
        let accept_key = compute_accept_key(client_key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {}\r\n\
             X-Protocol-Support: websocket,http11,http2,grpc\r\n\
             X-Allowed-Methods: {}\r\n\
             X-Multistatus: 207 Multi-Status\r\n\
             X-VPN-Enabled: {}\r\n\
             \r\n",
            accept_key,
            cfg.allow_methods.join(", "),
            cfg.vpn_enabled
        );
        
        client.write_all(response.as_bytes()).await?;
        info!("🔗 WSS (WebSocket Secure) upgrade OK, conectando ao SSH...");
        
        // Conecta ao destino com VPN
        let remote = match connect_with_vpn(&cfg.default_target, &cfg).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("❌ Falha ao conectar ao SSH: {}", e);
                bail!("Falha ao conectar ao SSH: {}", e);
            }
        };
        
        info!("✅ Conectado ao {} via {}", cfg.default_target, 
            if cfg.vpn_enabled { "VPN" } else { "rede local" });
        
        let (mut client_reader, mut client_writer) = client.into_split();
        let (mut remote_reader, mut remote_writer) = remote.into_split();
        
        tokio::try_join!(
            tokio::io::copy(&mut client_reader, &mut remote_writer),
            tokio::io::copy(&mut remote_reader, &mut client_writer)
        )?;
        
        info!("🔚 Conexão WSS->SSH encerrada");
        return Ok(());
    }
    
    // Resposta para requisição HTTP normal
    let content = format!(
        "Método: {}\n\
         Host: {}\n\
         Path: {}\n\
         Protocolo: TLS/HTTP/1.1\n\
         VPN: {}\n\
         Métodos permitidos: {}\n\
         Headers recebidos: {}\n\
         Status: {}",
        method,
        host,
        path,
        if cfg.vpn_enabled { "Ativada" } else { "Desativada" },
        cfg.allow_methods.join(", "),
        headers_map.len(),
        cfg.status
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
         X-Host: {}\r\n\
         \r\n\
         {}",
        content.len(),
        cfg.allow_methods.join(", "),
        method,
        cfg.vpn_enabled,
        host,
        content
    );
    
    client.write_all(response.as_bytes()).await?;
    info!("📨 Resposta TLS enviada para método {}", method);
    Ok(())
}

/// Versão simplificada para compatibilidade com o código existente
pub async fn handle_tls_legacy(client: TcpStream) -> Result<()> {
    handle_tls(client, None).await
}
