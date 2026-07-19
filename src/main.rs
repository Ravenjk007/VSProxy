use std::env;
use tokio::net::{TcpListener, TcpStream};
use std::collections::HashMap;

mod socks;
mod wsproxy;

/// Configuração global do proxy, lida a partir dos argumentos de linha de comando.
#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub status: String, // texto enviado na resposta HTTP fake (ex: "@VSProxy")
    pub default_target: String, // destino padrão pros modos websocket/direct (ex: SSH local)
    pub vpn_enabled: bool,
    pub vpn_bind: String,
    pub vpn_proxy: Option<String>,
    pub allow_methods: Vec<String>, // Métodos HTTP permitidos
}

#[tokio::main]
async fn main() {
    let config = parse_args();
    let listener = TcpListener::bind(("0.0.0.0", config.port))
        .await
        .expect("Falha ao abrir a porta. Ela já está em uso?");

    println!("VSProxy escutando na porta {}", config.port);
    println!("Destino padrão: {}", config.default_target);
    println!("Métodos permitidos: {:?}", config.allow_methods);
    if config.vpn_enabled {
        println!("🔐 VPN ativada (bind: {})", config.vpn_bind);
        if let Some(proxy) = &config.vpn_proxy {
            println!("🔗 Proxy configurado: {}", proxy);
        }
    }

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

/// Lê argumentos com suporte a VPN e métodos HTTP
/// --port 80 --status "@VSProxy" --target 127.0.0.1:22 
/// --vpn --vpn-bind 10.0.0.1:8080 --vpn-proxy proxy.example.com:1080
/// --methods GET,POST,PUT,DELETE,CONNECT,HEAD,PATCH,OPTIONS,TRACE,ACL,MOVE
fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();
    let mut port: u16 = 80;
    let mut status = "@VSProxy".to_string();
    let mut default_target = "127.0.0.1:22".to_string();
    let mut vpn_enabled = false;
    let mut vpn_bind = "0.0.0.0:0".to_string();
    let mut vpn_proxy = None;
    let mut allow_methods = vec![
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
    ];

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
            "--vpn" => {
                vpn_enabled = true;
                i += 1;
            }
            "--vpn-bind" => {
                vpn_bind = args.get(i + 1).cloned().unwrap_or(vpn_bind);
                i += 2;
            }
            "--vpn-proxy" => {
                vpn_proxy = args.get(i + 1).cloned();
                i += 2;
            }
            "--methods" => {
                if let Some(methods_str) = args.get(i + 1) {
                    allow_methods = methods_str
                        .split(',')
                        .map(|s| s.trim().to_uppercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    Config {
        port,
        status,
        default_target,
        vpn_enabled,
        vpn_bind,
        vpn_proxy,
        allow_methods,
    }
}

/// Decide qual protocolo tratar de acordo com os primeiros bytes recebidos.
async fn handle_client(socket: TcpStream, cfg: Config) -> std::io::Result<()> {
    let mut peek_buf = [0u8; 12]; // Aumentado para capturar mais métodos
    let n = socket.peek(&mut peek_buf).await?;

    if n >= 1 && peek_buf[0] == 0x05 {
        // Primeiro byte 0x05 = início de handshake SOCKS5
        socks::handle_socks5(socket).await
    } else if n >= 3 && &peek_buf[0..3] == b"GET" {
        // Requisição GET
        wsproxy::handle_multiprotocol_request(socket, &cfg, "GET").await
    } else if n >= 4 && &peek_buf[0..4] == b"POST" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "POST").await
    } else if n >= 3 && &peek_buf[0..3] == b"PUT" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "PUT").await
    } else if n >= 6 && &peek_buf[0..6] == b"DELETE" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "DELETE").await
    } else if n >= 7 && &peek_buf[0..7] == b"CONNECT" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "CONNECT").await
    } else if n >= 4 && &peek_buf[0..4] == b"HEAD" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "HEAD").await
    } else if n >= 5 && &peek_buf[0..5] == b"PATCH" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "PATCH").await
    } else if n >= 7 && &peek_buf[0..7] == b"OPTIONS" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "OPTIONS").await
    } else if n >= 5 && &peek_buf[0..5] == b"TRACE" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "TRACE").await
    } else if n >= 3 && &peek_buf[0..3] == b"ACL" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "ACL").await
    } else if n >= 4 && &peek_buf[0..4] == b"MOVE" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "MOVE").await
    } else if n >= 4 && &peek_buf[0..4] == b"COPY" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "COPY").await
    } else if n >= 4 && &peek_buf[0..4] == b"LINK" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "LINK").await
    } else if n >= 6 && &peek_buf[0..6] == b"UNLINK" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "UNLINK").await
    } else if n >= 5 && &peek_buf[0..5] == b"PURGE" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "PURGE").await
    } else if n >= 4 && &peek_buf[0..4] == b"LOCK" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "LOCK").await
    } else if n >= 6 && &peek_buf[0..6] == b"UNLOCK" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "UNLOCK").await
    } else if n >= 8 && &peek_buf[0..8] == b"PROPFIND" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "PROPFIND").await
    } else if n >= 4 && &peek_buf[0..4] == b"VIEW" {
        wsproxy::handle_multiprotocol_request(socket, &cfg, "VIEW").await
    } else {
        // Qualquer outra coisa: modo "security" (resposta direta, sem esperar handshake)
        wsproxy::handle_direct(socket, &cfg).await
    }
}
