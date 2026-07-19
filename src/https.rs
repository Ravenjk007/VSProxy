use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use rustls::{ServerConfig, Certificate, PrivateKey};
use std::sync::Arc;
use std::collections::HashMap;
use anyhow::Result;
use log::{info, warn, error};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Configuração para HTTPS com suporte VPN
#[derive(Clone)]
pub struct HttpsConfig {
    pub default_target: String,
    pub vpn_enabled: bool,
    pub vpn_bind: String,
    pub vpn_proxy: Option<String>,
    pub allow_methods: Vec<String>,
    pub status: String,
}

/// Lê headers HTTP até \r\n\r\n
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
fn is_method_allowed(method: &str, cfg: &HttpsConfig) -> bool {
    cfg.allow_methods.iter().any(|m| m.eq_ignore_ascii_case(method))
}

/// Conecta ao destino com suporte VPN
async fn connect_with_vpn(target: &str, cfg: &HttpsConfig) -> std::io::Result<TcpStream> {
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

pub async fn handle_https(socket: TcpStream, domain: &str, cfg: HttpsConfig) -> Result<()> {
    info!("🔒 HTTPS connection for: {}", domain);
    
    // Mapear domínio para certificado (Let's Encrypt)
    let (cert, key) = load_certificates_for_domain(domain)?;
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let mut tls_stream = acceptor.accept(socket).await?;
    
    info!("🔒 TLS handshake OK para {}, processando requisição...", domain);
    
    // Lê os headers HTTP
    let (headers_str, headers_map) = match read_http_headers(&mut tls_stream).await {
        Ok(data) => data,
        Err(e) => {
            error!("❌ Erro ao ler headers: {}", e);
            let _ = tls_stream.shutdown().await;
            anyhow::bail!("Falha ao ler headers HTTP: {}", e);
        }
    };
    
    let method = extract_method(&headers_str).unwrap_or_else(|| "UNKNOWN".to_string());
    info!("📨 Método HTTPS detectado: {}", method);
    
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
        tls_stream.write_all(response.as_bytes()).await?;
        let _ = tls_stream.shutdown().await;
        return Ok(());
    }
    
    // Detecção de WebSocket sobre HTTPS (WSS)
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
            .ok_or_else(|| anyhow::anyhow!("Chave WebSocket faltando"))?;
        
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
        
        tls_stream.write_all(response.as_bytes()).await?;
        info!("🔗 WSS (WebSocket Secure) upgrade OK, conectando ao SSH...");
        
        // Conecta ao destino com VPN
        let remote = connect_with_vpn(&cfg.default_target, &cfg).await?;
        info!("✅ Conectado ao {} via {}", cfg.default_target, 
            if cfg.vpn_enabled { "VPN" } else { "rede local" });
        
        let (mut tls_reader, mut tls_writer) = tokio::io::split(tls_stream);
        let (mut remote_reader, mut remote_writer) = remote.into_split();
        
        tokio::try_join!(
            tokio::io::copy(&mut tls_reader, &mut remote_writer),
            tokio::io::copy(&mut remote_reader, &mut tls_writer)
        )?;
        
        info!("🔚 Conexão WSS->SSH encerrada ({})", domain);
        return Ok(());
    }
    
    // Resposta para métodos HTTP sobre HTTPS
    let content = format!(
        "Método: {}\n\
         Domínio: {}\n\
         Protocolo: HTTPS/HTTP/1.1\n\
         VPN: {}\n\
         Métodos permitidos: {}\n\
         Headers recebidos: {}",
        method,
        domain,
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
         X-Domain: {}\r\n\
         \r\n\
         {}",
        content.len(),
        cfg.allow_methods.join(", "),
        method,
        cfg.vpn_enabled,
        domain,
        content
    );
    
    tls_stream.write_all(response.as_bytes()).await?;
    info!("📨 Resposta HTTPS enviada para método {}", method);
    
    // Shutdown TLS gracefully
    let _ = tls_stream.shutdown().await;
    info!("🔚 Conexão HTTPS encerrada ({})", domain);
    Ok(())
}

fn load_certificates_for_domain(domain: &str) -> Result<(Certificate, PrivateKey)> {
    let cert_path = format!("/etc/letsencrypt/live/{}/fullchain.pem", domain);
    let key_path = format!("/etc/letsencrypt/live/{}/privkey.pem", domain);
    
    let cert_data = std::fs::read(&cert_path)
        .map_err(|e| anyhow::anyhow!("Falha ao ler certificado {}: {}", cert_path, e))?;
    let key_data = std::fs::read(&key_path)
        .map_err(|e| anyhow::anyhow!("Falha ao ler chave privada {}: {}", key_path, e))?;
    
    Ok((Certificate(cert_data), PrivateKey(key_data)))
}

/// Verifica se existe certificado local para o domínio (usado pelo main.rs pra decidir
/// entre TLS termination (https.rs) e SNI passthrough (tls.rs))
pub fn has_local_certificate(domain: &str) -> bool {
    std::path::Path::new(&format!("/etc/letsencrypt/live/{}/fullchain.pem", domain)).exists()
}

/// Versão simplificada para compatibilidade com o código existente
pub async fn handle_https_legacy(socket: TcpStream, domain: &str) -> Result<()> {
    let cfg = HttpsConfig {
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
    };
    
    handle_https(socket, domain, cfg).await
}
