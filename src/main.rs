mod socks5;
mod tls;
mod https;
mod websocket;
mod http_handler;
mod tcp_fallback;
mod security;

use tokio::net::TcpListener;
use clap::Parser;
use anyhow::Result;
use log::{info, error};

#[derive(Parser)]
#[command(name = "vsproxy")]
#[command(about = "Multiprotocol proxy server")]
struct Cli {
    #[arg(short = 'p', long = "port", default_value = "8080")]
    port: u16,
    #[arg(short = 'd', long = "debug")]
    debug: bool,
}

const PEEK_SIZE: usize = 8192;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.debug {
        env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("🚀 VSProxy listening on {}", addr);
    info!("📡 Protocols: SOCKS5, TLS, HTTPS, WebSocket, SECURITY, TCP");

    if std::env::var("VSPROXY_TOKEN").is_err() {
        info!("⚠️  VSPROXY_TOKEN não definido — protocolo SECURITY vai aceitar sem autenticação!");
    }

    while let Ok((socket, peer_addr)) = listener.accept().await {
        tokio::spawn(async move {
            let mut buf = vec![0u8; PEEK_SIZE];
            match socket.peek(&mut buf).await {
                Ok(n) if n > 0 => {
                    let buf = &buf[..n];

                    let result: Result<()> = if buf[0] == 0x05 {
                        info!("🔐 SOCKS5 <- {}", peer_addr);
                        socks5::handle_socks5(socket).await
                    } else if buf[0] == 0x16 {
                        // TLS ClientHello: decide entre HTTPS (cert próprio) e passthrough por SNI
                        match tls::extract_sni(buf) {
                            Some(domain) if https::has_local_certificate(&domain) => {
                                info!("🔒 HTTPS (cert local) <- {} [{}]", peer_addr, domain);
                                https::handle_https(socket, &domain).await
                            }
                            Some(domain) => {
                                info!("🔒 TLS passthrough <- {} [{}]", peer_addr, domain);
                                tls::handle_tls(socket).await
                            }
                            None => {
                                info!("🔒 TLS sem SNI <- {}, tentando passthrough mesmo assim", peer_addr);
                                tls::handle_tls(socket).await
                            }
                        }
                    } else {
                        let data_str = String::from_utf8_lossy(buf);

                        let is_http = data_str.starts_with("GET ")
                            || data_str.starts_with("POST ")
                            || data_str.starts_with("PUT ")
                            || data_str.starts_with("DELETE ")
                            || data_str.starts_with("PATCH ")
                            || data_str.starts_with("HEAD ")
                            || data_str.starts_with("CONNECT ")
                            || data_str.starts_with("OPTIONS ")
                            || data_str.starts_with("TRACE ")
                            || data_str.starts_with("HTTP/");

                        if is_http {
                            let is_websocket_upgrade = data_str
                                .to_lowercase()
                                .contains("upgrade: websocket");

                            if is_websocket_upgrade {
                                info!("🌐 WebSocket <- {}", peer_addr);
                                websocket::handle_websocket(socket).await
                            } else {
                                info!("🌐 HTTP <- {}", peer_addr);
                                http_handler::handle(socket).await
                            }
                        } else if data_str.starts_with("SECURITY") || data_str.starts_with("AUTH") {
                            info!("🔐 SECURITY <- {}", peer_addr);
                            security::handle_security(socket).await
                        } else {
                            info!("📦 TCP fallback <- {}", peer_addr);
                            tcp_fallback::handle_tcp(socket).await
                        }
                    };

                    if let Err(e) = result {
                        error!("Handler error for {}: {:?}", peer_addr, e);
                    }
                }
                Ok(_) => info!("Connection closed by {}", peer_addr),
                Err(e) => error!("Peek error: {}", e),
            }
        });
    }
    Ok(())
}
