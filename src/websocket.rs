use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::Result;
use log::{info, warn};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

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

pub async fn handle_websocket(mut socket: TcpStream) -> Result<()> {
    info!("🌐 WebSocket/HTTP handshake...");

    let headers = read_http_headers(&mut socket).await?;

    // Se não tiver Sec-WebSocket-Key, é HTTP comum, não upgrade de WS.
    // Aqui você decide: encaminhar como HTTP normal ou recusar.
    let client_key = match extract_header(&headers, "Sec-WebSocket-Key") {
        Some(k) => k.to_string(),
        None => {
            warn!("⚠️ Requisição HTTP sem Sec-WebSocket-Key — não é upgrade WS");
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            socket.write_all(response.as_bytes()).await?;
            anyhow::bail!("Missing Sec-WebSocket-Key header");
        }
    };

    let accept_key = compute_accept_key(&client_key);

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept_key
    );

    socket.write_all(response.as_bytes()).await?;
    info!("🌐 WebSocket handshake OK! Encaminhando para SSH...");

    // TODO: tornar configurável via CLI em vez de hardcoded
    let target = "127.0.0.1:22";

    match TcpStream::connect(target).await {
        Ok(remote) => {
            info!("✅ Conectado ao SSH na porta 22");
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
            info!("❌ Falha ao conectar ao SSH: {}", e);
            anyhow::bail!("SSH connection failed: {}", e)
        }
    }
}
