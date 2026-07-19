use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::{Result, bail};
use log::{info, warn};
use std::env;

/// Token esperado. Configure via variável de ambiente VSPROXY_TOKEN.
/// Se não for definida, autenticação fica DESABILITADA (aceita qualquer um) —
/// defina sempre em produção.
fn expected_token() -> Option<String> {
    env::var("VSPROXY_TOKEN").ok()
}

/// Lê byte a byte até \n, retorna a linha (sem o \r\n final)
async fn read_line(socket: &mut TcpStream) -> Result<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 1];
    loop {
        socket.read_exact(&mut tmp).await?;
        if tmp[0] == b'\n' { break; }
        buf.push(tmp[0]);
        if buf.len() > 2048 { break; }
    }
    let s = String::from_utf8_lossy(&buf).to_string();
    Ok(s.trim_end_matches('\r').to_string())
}

pub async fn handle_security(mut client: TcpStream) -> Result<()> {
    info!("🔐 SECURITY handshake...");

    let line = read_line(&mut client).await?;
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    let token = parts.get(1).unwrap_or(&"").trim().to_string();

    match expected_token() {
        Some(expected) => {
            if token != expected {
                warn!("🔐 Token inválido, recusando conexão");
                client.write_all(b"DENIED\r\n").await?;
                bail!("Invalid security token");
            }
        }
        None => {
            warn!("⚠️ BSPROXY_TOKEN não definido — SECURITY está aceitando sem validar!");
        }
    }

    client.write_all(b"OK\r\n").await?;
    info!("🔐 SECURITY autenticado, encaminhando para SSH...");

    // TODO: tornar configurável via CLI em vez de hardcoded
    let target = "127.0.0.1:22";

    match TcpStream::connect(target).await {
        Ok(remote) => {
            let (mut client_reader, mut client_writer) = client.into_split();
            let (mut remote_reader, mut remote_writer) = remote.into_split();

            tokio::try_join!(
                tokio::io::copy(&mut client_reader, &mut remote_writer),
                tokio::io::copy(&mut remote_reader, &mut client_writer)
            )?;

            info!("🔚 Conexão SECURITY encerrada");
            Ok(())
        }
        Err(e) => {
            bail!("Falha ao conectar ao alvo: {}", e);
        }
    }
}
