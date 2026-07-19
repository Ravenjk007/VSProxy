use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use rustls::{ServerConfig, Certificate, PrivateKey};
use std::sync::Arc;
use anyhow::Result;
use log::info;

pub async fn handle_https(socket: TcpStream, domain: &str) -> Result<()> {
    info!("🔒 HTTPS connection for: {}", domain);

    // Mapear domínio para certificado (Let's Encrypt)
    let (cert, key) = load_certificates_for_domain(domain)?;

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let mut tls_stream = acceptor.accept(socket).await?;

    info!("🔒 TLS handshake OK para {}, encaminhando para SSH...", domain);

    // TODO: tornar configurável via CLI em vez de hardcoded
    let target = "127.0.0.1:22";

    match TcpStream::connect(target).await {
        Ok(mut remote) => {
            let (mut remote_reader, mut remote_writer) = remote.split();
            let (mut tls_reader, mut tls_writer) = tokio::io::split(tls_stream);

            tokio::try_join!(
                tokio::io::copy(&mut tls_reader, &mut remote_writer),
                tokio::io::copy(&mut remote_reader, &mut tls_writer)
            )?;

            info!("🔚 Conexão HTTPS->SSH encerrada ({})", domain);
            Ok(())
        }
        Err(e) => {
            let _ = tls_stream.shutdown().await;
            anyhow::bail!("Falha ao conectar ao SSH: {}", e);
        }
    }
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
