use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::Config;

/// Lê e descarta os headers HTTP até encontrar a linha em branco (\r\n\r\n).
async fn consume_http_headers(socket: &mut TcpStream) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 1];

    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);

        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        // Proteção simples contra headers gigantes/mal-formados
        if buf.len() > 8192 {
            break;
        }
    }
    Ok(())
}

/// Modo Websocket: consome o handshake HTTP enviado pelo cliente,
/// responde com um "upgrade" (na prática, um texto fixo pra enganar
/// inspeção de tráfego) e então encaminha os bytes crus pro destino.
pub async fn handle_websocket(mut socket: TcpStream, cfg: &Config) -> std::io::Result<()> {
    consume_http_headers(&mut socket).await?;

    let response = format!("HTTP/1.1 101 {}\r\n\r\n", cfg.status);
    socket.write_all(response.as_bytes()).await?;

    forward_to_target(socket, &cfg.default_target).await
}

/// Modo "Security": não espera nenhum handshake HTTP. Manda a linha de
/// status direto e encaminha a conexão crua. Útil para clientes que já
/// mandam a conexão (ex: sshd) sem passar por um proxy HTTP antes.
pub async fn handle_direct(mut socket: TcpStream, cfg: &Config) -> std::io::Result<()> {
    let response = format!("HTTP/1.1 200 {}\r\n\r\n", cfg.status);
    socket.write_all(response.as_bytes()).await?;

    forward_to_target(socket, &cfg.default_target).await
}

async fn forward_to_target(mut client: TcpStream, target: &str) -> std::io::Result<()> {
    let mut remote = TcpStream::connect(target).await?;
    copy_bidirectional(&mut client, &mut remote).await?;
    Ok(())
}
