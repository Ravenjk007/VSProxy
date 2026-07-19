use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Implementação mínima de um servidor SOCKS5 (RFC 1928), sem autenticação,
/// suportando apenas o comando CONNECT (o mais comum).
pub async fn handle_socks5(mut client: TcpStream) -> std::io::Result<()> {
    // --- Etapa 1: negociação do método de autenticação ---
    let mut header = [0u8; 2];
    client.read_exact(&mut header).await?;
    let nmethods = header[1] as usize;

    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;

    // Respondemos dizendo que não exigimos autenticação (0x00)
    client.write_all(&[0x05, 0x00]).await?;

    // --- Etapa 2: requisição de conexão ---
    let mut req = [0u8; 4];
    client.read_exact(&mut req).await?;
    let cmd = req[1];
    let atyp = req[3];

    let target_addr = match atyp {
        0x01 => {
            // Endereço IPv4
            let mut addr = [0u8; 4];
            client.read_exact(&mut addr).await?;
            let port = read_port(&mut client).await?;
            format!("{}.{}.{}.{}:{}", addr[0], addr[1], addr[2], addr[3], port)
        }
        0x03 => {
            // Nome de domínio
            let mut len_buf = [0u8; 1];
            client.read_exact(&mut len_buf).await?;
            let len = len_buf[0] as usize;

            let mut domain = vec![0u8; len];
            client.read_exact(&mut domain).await?;
            let port = read_port(&mut client).await?;
            format!("{}:{}", String::from_utf8_lossy(&domain), port)
        }
        0x04 => {
            // Endereço IPv6
            let mut addr = [0u8; 16];
            client.read_exact(&mut addr).await?;
            let port = read_port(&mut client).await?;
            let segs: Vec<String> = addr
                .chunks(2)
                .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
                .collect();
            format!("[{}]:{}", segs.join(":"), port)
        }
        _ => {
            send_reply(&mut client, 0x08).await?; // tipo de endereço não suportado
            return Ok(());
        }
    };

    if cmd != 0x01 {
        // Só suportamos CONNECT. BIND e UDP ASSOCIATE ficam de fora por enquanto.
        send_reply(&mut client, 0x07).await?; // comando não suportado
        return Ok(());
    }

    match TcpStream::connect(&target_addr).await {
        Ok(mut remote) => {
            send_reply(&mut client, 0x00).await?; // sucesso
            copy_bidirectional(&mut client, &mut remote).await?;
            Ok(())
        }
        Err(e) => {
            send_reply(&mut client, 0x05).await?; // conexão recusada
            Err(e)
        }
    }
}

async fn read_port(client: &mut TcpStream) -> std::io::Result<u16> {
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    Ok(u16::from_be_bytes(port_buf))
}

/// Envia uma resposta SOCKS5 padrão (endereço vinculado simplificado como 0.0.0.0:0).
async fn send_reply(client: &mut TcpStream, code: u8) -> std::io::Result<()> {
    client
        .write_all(&[0x05, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
}
