use std::env;
use tokio::net::{TcpListener, TcpStream};

mod socks;
mod wsproxy;

/// Configuração global do proxy, lida a partir dos argumentos de linha de comando.
#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub status: String,         // texto enviado na resposta HTTP fake (ex: "@BSProxy")
    pub default_target: String, // destino padrão pros modos websocket/direct (ex: SSH local)
}

#[tokio::main]
async fn main() {
    let config = parse_args();

    let listener = TcpListener::bind(("0.0.0.0", config.port))
        .await
        .expect("Falha ao abrir a porta. Ela já está em uso?");

    println!("BSProxy escutando na porta {}", config.port);
    println!("Destino padrão: {}", config.default_target);

    loop {
        let (socket, addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Erro ao aceitar conexão: {}", e);
                continue;
            }
        };

        let cfg = config.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, cfg).await {
                eprintln!("Conexão de {} encerrada: {}", addr, e);
            }
        });
    }
}

/// Lê argumentos como: --port 80 --status "@BSProxy" --target 127.0.0.1:22
fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();

    let mut port: u16 = 80;
    let mut status = "@BSProxy".to_string();
    let mut default_target = "127.0.0.1:22".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(port);
                i += 2;
            }
            "--status" => {
                status = args.get(i + 1).cloned().unwrap_or(status);
                i += 2;
            }
            "--target" => {
                default_target = args.get(i + 1).cloned().unwrap_or(default_target);
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    Config { port, status, default_target }
}

/// Decide qual protocolo tratar de acordo com os primeiros bytes recebidos.
async fn handle_client(socket: TcpStream, cfg: Config) -> std::io::Result<()> {
    let mut peek_buf = [0u8; 8];
    let n = socket.peek(&mut peek_buf).await?;

    if n >= 1 && peek_buf[0] == 0x05 {
        // Primeiro byte 0x05 = início de handshake SOCKS5
        socks::handle_socks5(socket).await
    } else if n >= 3 && &peek_buf[0..3] == b"GET" {
        // Parece uma requisição HTTP -> tratamos como upgrade Websocket
        wsproxy::handle_websocket(socket, &cfg).await
    } else {
        // Qualquer outra coisa: modo "security" (resposta direta, sem esperar handshake)
        wsproxy::handle_direct(socket, &cfg).await
    }
}
