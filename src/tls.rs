use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use anyhow::{Result, bail};
use log::{info, warn};

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
    if pos >= data.len() { return None; }

    // session id
    let session_id_len = *data.get(pos)? as usize;
    pos += 1 + session_id_len;
    if pos + 2 > data.len() { return None; }

    // cipher suites
    let cipher_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2 + cipher_len;
    if pos >= data.len() { return None; }

    // compression methods
    let comp_len = *data.get(pos)? as usize;
    pos += 1 + comp_len;
    if pos + 2 > data.len() { return None; }

    // extensions total length
    let ext_total_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    let ext_end = (pos + ext_total_len).min(data.len());

    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let ext_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > data.len() { break; }

        if ext_type == 0x0000 {
            // server_name extension
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

pub async fn handle_tls(mut client: TcpStream) -> Result<()> {
    info!("🔒 TLS handshake, lendo ClientHello...");

    let hello = read_client_hello(&mut client).await?;
    let sni = extract_sni(&hello);

    let target = match &sni {
        Some(host) => {
            info!("🔒 TLS SNI -> {}", host);
            format!("{}:443", host)
        }
        None => {
            warn!("⚠️ Sem SNI no ClientHello, não é possível rotear");
            bail!("No SNI found in ClientHello");
        }
    };

    match TcpStream::connect(&target).await {
        Ok(mut remote) => {
            // Reenviar o ClientHello que já consumimos, antes de fazer o splice
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
        Err(e) => {
            bail!("Falha ao conectar em {}: {}", target, e);
        }
    }
}
