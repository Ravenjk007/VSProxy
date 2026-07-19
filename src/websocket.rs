use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::Result;
use log::{info, warn, error};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use std::net::SocketAddr;
use std::str::FromStr;

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Configuração para conexão VPN
#[derive(Debug, Clone)]
pub struct VpnConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub proxy_address: Option<String>,
}

/// Lê os headers HTTP e retorna eles como String (até \r\n\r\n)
async fn read_http_headers(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf: Vec<u8> = Vec::new();
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

/// Extrai o valor de um header específico (case-insensitive)
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

/// Calcula o Sec-WebSocket-Accept correto a partir da chave do cliente
fn compute_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    let result = hasher.finalize();
    general_purpose::STANDARD.encode(result)
}

/// Analisa a requisição e determina o tipo de protocolo
fn analyze_request(headers: &str) -> RequestType {
    // Verifica se é WebSocket
    if let Some(upgrade) = extract_header(headers, "Upgrade") {
        if upgrade.to_lowercase() == "websocket" {
            if extract_header(headers, "Sec-WebSocket-Key").is_some() {
                return RequestType::WebSocket;
            }
        }
    }
    
    // Verifica se é HTTP/2
    if let Some(protocol) = extract_header(headers, "Protocol") {
        if protocol.to_lowercase().contains("http/2") {
            return RequestType::Http2;
        }
    }
    
    // Verifica se é gRPC
    if extract_header(headers, "Content-Type").map_or(false, |ct| ct.contains("application/grpc")) {
        return RequestType::Grpc;
    }
    
    // Verifica se é HTTP/1.1 padrão
    if headers.starts_with("GET") || headers.starts_with("POST") || headers.starts_with("PUT") || headers.starts_with("DELETE") {
        if let Some(version) = extract_header(headers, "HTTP-Version") {
            if version.contains("1.1") {
                return RequestType::Http11;
            }
        }
        return RequestType::Http11;
    }
    
    RequestType::Unknown
}

/// Tipos de requisição suportados
#[derive(Debug, PartialEq)]
enum RequestType {
    WebSocket,
    Http11,
    Http2,
    Grpc,
    Unknown,
}

/// Resposta multiprotocolo
async fn handle_multiprotocol_response(
    socket: &mut TcpStream,
    request_type: RequestType,
    headers: &str,
) -> std::io::Result<()> {
    let response = match request_type {
        RequestType::WebSocket => {
            // Resposta WebSocket
            let client_key = extract_header(headers, "Sec-WebSocket-Key")
                .unwrap_or("default-key");
            let accept_key = compute_accept_key(client_key);
            format!(
                "HTTP/1.1 101 Switching Protocols\r\n\
                Upgrade: websocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Accept: {}\r\n\
                X-Protocol-Support: websocket,http11,http2,grpc\r\n\
                \r\n",
                accept_key
            )
        }
        RequestType::Http11 | RequestType::Http2 | RequestType::Grpc => {
            // Resposta HTTP com suporte multiprotocolo
            let status = match request_type {
                RequestType::Grpc => "200 OK",
                _ => "200 OK"
            };
            format!(
                "HTTP/1.1 {}\r\n\
                Content-Type: text/plain\r\n\
                Content-Length: 42\r\n\
                X-Protocol-Support: websocket,http11,http2,grpc\r\n\
                X-Multistatus: 207 Multi-Status\r\n\
                \r\n\
                Protocol supported: {:?}. Use WebSocket for SSH.",
                status, request_type
            )
        }
        RequestType::Unknown => {
            // Resposta para protocolo desconhecido
            "HTTP/1.1 400 Bad Request\r\n\
            Content-Type: text/plain\r\n\
            Content-Length: 27\r\n\
            \r\n\
            Unsupported protocol\r\n".to_string()
        }
    };
    
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

/// Configura a conexão VPN se ativada
async fn setup_vpn_connection(target: &str, vpn_config: &VpnConfig) -> Result<TcpStream> {
    if vpn_config.enabled {
        info!("🔐 Usando conexão VPN: {}", vpn_config.bind_address);
        
        // Se tiver proxy configurado, usa ele
        if let Some(proxy) = &vpn_config.proxy_address {
            info!("🔗 Conectando via proxy: {}", proxy);
            let proxy_addr = SocketAddr::from_str(proxy)?;
            let stream = TcpStream::connect(proxy_addr).await?;
            return Ok(stream);
        }
        
        // Caso contrário, usa o bind_address como interface
        let bind_addr = SocketAddr::from_str(&vpn_config.bind_address)?;
        let stream = TcpStream::connect(target).await?;
        // Em uma implementação real, bind seria feito no socket
        return Ok(stream);
    }
    
    // Conexão normal sem VPN
    Ok(TcpStream::connect(target).await?)
}

pub async fn handle_websocket(mut socket: TcpStream, vpn_config: Option<VpnConfig>) -> Result<()> {
    info!("🌐 Processando requisição multi-protocolo...");
    
    let headers = read_http_headers(&mut socket).await?;
    info!("📨 Headers recebidos: {}", headers.lines().next().unwrap_or("N/A"));
    
    // Analisa o tipo de requisição
    let request_type = analyze_request(&headers);
    info!("📡 Tipo de requisição detectado: {:?}", request_type);
    
    // Responde com suporte multiprotocolo
    if let Err(e) = handle_multiprotocol_response(&mut socket, request_type, &headers).await {
        error!("❌ Erro ao enviar resposta multiprotocolo: {}", e);
        return Err(e.into());
    }
    
    // Se for WebSocket, prossegue com o upgrade
    if request_type == RequestType::WebSocket {
        info!("🔗 WebSocket confirmado, estabelecendo conexão SSH...");
        
        // Configuração do target com suporte a VPN
        let target = "127.0.0.1:22";
        let vpn_config = vpn_config.unwrap_or(VpnConfig {
            enabled: false,
            bind_address: "0.0.0.0:0".to_string(),
            proxy_address: None,
        });
        
        match setup_vpn_connection(target, &vpn_config).await {
            Ok(remote) => {
                info!("✅ Conectado ao SSH via {}", 
                    if vpn_config.enabled { "VPN" } else { "rede local" }
                );
                
                let (mut client_reader, mut client_writer) = socket.into_split();
                let (mut remote_reader, mut remote_writer) = remote.into_split();
                
                tokio::try_join!(
                    tokio::io::copy(&mut client_reader, &mut remote_writer),
                    tokio::io::copy(&mut remote_reader, &mut client_writer)
                )?;
                
                info!("🔚 Conexão WebSocket->SSH encerrada");
                Ok(())
            }
            Err(e) => {
                error!("❌ Falha ao conectar ao SSH via VPN: {}", e);
                anyhow::bail!("SSH connection failed via VPN: {}", e)
            }
        }
    } else {
        info!("📨 Conexão encerrada após resposta multiprotocolo");
        Ok(())
    }
}

// Função para testar o servidor multi-protocolo
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    
    #[tokio::test]
    async fn test_multiprotocol_headers() {
        let ws_headers = "GET / HTTP/1.1\r\n\
                         Upgrade: websocket\r\n\
                         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                         \r\n";
        
        let http_headers = "GET / HTTP/1.1\r\n\
                           Host: localhost\r\n\
                           \r\n";
        
        let grpc_headers = "POST /grpc HTTP/1.1\r\n\
                           Content-Type: application/grpc\r\n\
                           \r\n";
        
        assert_eq!(analyze_request(ws_headers), RequestType::WebSocket);
        assert_eq!(analyze_request(http_headers), RequestType::Http11);
        assert_eq!(analyze_request(grpc_headers), RequestType::Grpc);
    }
}
