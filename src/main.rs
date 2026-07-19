#!/bin/bash
# VSProxy Complete Installer - Versão Funcional
# Com suporte a SOCKS5, WebSocket e Security

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PORT=80
REDIRECT_PORT=8080

echo -e "${BLUE}========================================${NC}"
echo -e "${YELLOW}🚀 VSProxy Installer - Versão Completa${NC}"
echo -e "${BLUE}========================================${NC}"

# Verificar se é root
if [ "$EUID" -ne 0 ]; then 
    echo -e "${RED}❌ Execute como root (sudo).${NC}"
    exit 1
fi

# ============================================
# FUNÇÕES (mantidas as mesmas do seu script)
# ============================================
stop_conflicting_services() {
    echo -e "${YELLOW}🔍 Parando serviços conflitantes...${NC}"
    services=("apache2" "nginx" "lighttpd" "httpd")
    for service in "${services[@]}"; do
        systemctl stop $service 2>/dev/null
        systemctl disable $service 2>/dev/null
    done
    fuser -k 80/tcp 2>/dev/null
}

install_dependencies() {
    echo -e "${YELLOW}📦 Instalando dependências...${NC}"
    apt-get update -qq
    apt-get install -y -qq curl wget zip unzip build-essential pkg-config libssl-dev iptables net-tools lsof systemd git
}

install_rust() {
    if ! command -v cargo &> /dev/null; then
        echo -e "${YELLOW}🦀 Instalando Rust...${NC}"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source $HOME/.cargo/env
    fi
}

setup_port_redirect() {
    echo -e "${YELLOW}🔧 Configurando redirecionamento $PORT → $REDIRECT_PORT...${NC}"
    iptables -t nat -D PREROUTING -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT 2>/dev/null
    iptables -t nat -A PREROUTING -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT
    iptables -t nat -A OUTPUT -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT
    mkdir -p /etc/iptables
    iptables-save > /etc/iptables/rules.v4
}

# ============================================
# COMPILAR COM CÓDIGO COMPLETO
# ============================================
compile_and_install() {
    echo -e "${YELLOW}📥 Preparando código fonte completo...${NC}"
    cd /tmp
    rm -rf VSProxy VSProxy-main
    
    # Criar estrutura do projeto
    mkdir -p VSProxy-complete/src
    cd VSProxy-complete
    
    # Criar Cargo.toml
    cat > Cargo.toml << 'EOF'
[package]
name = "vsproxy"
version = "1.0.0"
edition = "2021"

[dependencies]
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
log = "0.4"
env_logger = "0.11"
sha1 = "0.10"
base64 = "0.21"
EOF

    # CRIAR main.rs COMPLETO (o código que eu forneci anteriormente)
    cat > src/main.rs << 'MAINEOF'
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{Result, anyhow};
use log::{info, error, warn, debug};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose, Engine as _};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use tokio::time::timeout;

// ============================================
// CONFIGURAÇÕES
// ============================================
pub struct Config {
    pub bind_addr: String,
    pub ssh_addr: String,
    pub connection_timeout: Duration,
    pub max_connections: usize,
    pub buffer_size: usize,
    pub valid_tokens: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".to_string(),
            ssh_addr: "127.0.0.1:22".to_string(),
            connection_timeout: Duration::from_secs(30),
            max_connections: 1000,
            buffer_size: 8192,
            valid_tokens: vec![
                "meu-token-seguro-123".to_string(),
                "admin-uuid-456".to_string()
            ],
        }
    }
}

// ============================================
// ESTATÍSTICAS
// ============================================
pub struct Stats {
    pub active_connections: AtomicUsize,
    pub total_websocket: AtomicUsize,
    pub total_socks5: AtomicUsize,
    pub total_security: AtomicUsize,
    pub total_errors: AtomicUsize,
    pub total_bytes_transferred: AtomicUsize,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_websocket: AtomicUsize::new(0),
            total_socks5: AtomicUsize::new(0),
            total_security: AtomicUsize::new(0),
            total_errors: AtomicUsize::new(0),
            total_bytes_transferred: AtomicUsize::new(0),
        }
    }
}

// ============================================
// HEADER PARSER
// ============================================
pub struct HttpHeaders {
    pub raw: String,
    parsed: std::collections::HashMap<String, String>,
}

impl HttpHeaders {
    pub fn new(raw: String) -> Self {
        let mut parsed = std::collections::HashMap::new();
        for line in raw.lines() {
            if let Some((k, v)) = line.split_once(':') {
                parsed.insert(k.trim().to_lowercase(), v.trim().to_string());
            }
        }
        Self { raw, parsed }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.parsed.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    pub fn is_websocket(&self) -> bool {
        self.raw.contains("Upgrade: websocket") || self.get("Sec-WebSocket-Key").is_some()
    }

    pub fn has_auth(&self) -> bool {
        self.get("X-Proxy-Token").is_some() || self.get("Authorization").is_some()
    }
}

// ============================================
// FUNÇÕES AUXILIARES
// ============================================
async fn read_http_headers(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 1];
    loop {
        socket.read_exact(&mut tmp).await?;
        buf.push(tmp[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 8192 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Headers too large"));
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

async fn read_http_headers_with_timeout(socket: &mut TcpStream, timeout_duration: Duration) -> std::io::Result<String> {
    timeout(timeout_duration, read_http_headers(socket))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "Timeout reading headers"))?
}

// ============================================
// PROTOCOLO WEBSOCKET
// ============================================
async fn handle_websocket(mut socket: TcpStream, headers: HttpHeaders, config: &Config) -> Result<()> {
    let client_key = headers.get("Sec-WebSocket-Key")
        .ok_or_else(|| anyhow!("Missing Sec-WebSocket-Key"))?;

    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept_key = general_purpose::STANDARD.encode(hasher.finalize());

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key
    );
    
    timeout(config.connection_timeout, socket.write_all(response.as_bytes()))
        .await
        .map_err(|_| anyhow!("Timeout writing websocket response"))??;
    
    forward_to_ssh(socket, config).await
}

// ============================================
// PROTOCOLO SOCKS5
// ============================================
async fn handle_socks5(mut socket: TcpStream, config: &Config) -> Result<()> {
    let mut buf = [0u8; 2];
    timeout(config.connection_timeout, socket.read_exact(&mut buf)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 handshake"))??;
    
    let nmethods = buf[1];
    let mut methods = vec![0u8; nmethods as usize];
    timeout(config.connection_timeout, socket.read_exact(&mut methods)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 methods"))??;
    
    socket.write_all(&[0x05, 0x00]).await?;

    let mut header = [0u8; 4];
    timeout(config.connection_timeout, socket.read_exact(&mut header)).await
        .map_err(|_| anyhow!("Timeout reading SOCKS5 request"))??;
    
    let target_addr = match header[3] {
        0x01 => {
            let mut addr = [0u8; 4];
            timeout(config.connection_timeout, socket.read_exact(&mut addr)).await
                .map_err(|_| anyhow!("Timeout reading IPv4 address"))??;
            format!("{}", Ipv4Addr::from(addr))
        }
        0x03 => {
            let len = timeout(config.connection_timeout, socket.read_u8()).await
                .map_err(|_| anyhow!("Timeout reading domain length"))??;
            let mut domain = vec![0u8; len as usize];
            timeout(config.connection_timeout, socket.read_exact(&mut domain)).await
                .map_err(|_| anyhow!("Timeout reading domain"))??;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => {
            let mut addr = [0u8; 16];
            timeout(config.connection_timeout, socket.read_exact(&mut addr)).await
                .map_err(|_| anyhow!("Timeout reading IPv6 address"))??;
            format!("{}", Ipv6Addr::from(addr))
        }
        _ => anyhow::bail!("Unsupported address type"),
    };
    
    let port = timeout(config.connection_timeout, socket.read_u16()).await
        .map_err(|_| anyhow!("Timeout reading port"))??;
    let target = format!("{}:{}", target_addr, port);
    debug!("SOCKS5 target: {}", target);
    
    match TcpStream::connect(&target).await {
        Ok(remote) => {
            socket.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            proxy_bridge(socket, remote, config).await
        }
        Err(e) => {
            error!("SOCKS5 connect failed: {}", e);
            socket.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("SOCKS5 connect failed")
        }
    }
}

// ============================================
// PROTOCOLO SECURITY
// ============================================
async fn handle_security(mut socket: TcpStream, headers: HttpHeaders, config: &Config) -> Result<()> {
    let token = headers.get("X-Proxy-Token")
        .or_else(|| headers.get("Authorization"))
        .map(|t| t.trim_start_matches("Bearer ").trim());

    if let Some(t) = token {
        if config.valid_tokens.contains(&t.to_string()) {
            socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
            return forward_to_ssh(socket, config).await;
        }
    }
    
    warn!("Unauthorized access attempt");
    socket.write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n").await?;
    anyhow::bail!("Unauthorized")
}

// ============================================
// CORE BRIDGE
// ============================================
async fn forward_to_ssh(socket: TcpStream, config: &Config) -> Result<()> {
    let remote = TcpStream::connect(&config.ssh_addr).await?;
    proxy_bridge(socket, remote, config).await
}

async fn proxy_bridge(client: TcpStream, remote: TcpStream, config: &Config) -> Result<()> {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut remote_read, mut remote_write) = remote.into_split();
    
    let client_to_remote = async {
        let mut buffer = vec![0u8; config.buffer_size];
        loop {
            match timeout(config.connection_timeout, client_read.read(&mut buffer)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => remote_write.write_all(&buffer[..n]).await?,
                Ok(Err(e)) => return Err(e),
                Err(_) => break,
            }
        }
        Ok::<_, std::io::Error>(())
    };

    let remote_to_client = async {
        let mut buffer = vec![0u8; config.buffer_size];
        loop {
            match timeout(config.connection_timeout, remote_read.read(&mut buffer)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => client_write.write_all(&buffer[..n]).await?,
                Ok(Err(e)) => return Err(e),
                Err(_) => break,
            }
        }
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = client_to_remote => result?,
        result = remote_to_client => result?,
    }
    Ok(())
}

// ============================================
// MAIN
// ============================================
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    
    let config = Config::default();
    let listener = TcpListener::bind(&config.bind_addr).await?;
    info!("🚀 VSProxy rodando em {}", config.bind_addr);
    info!("   SSH target: {}", config.ssh_addr);
    
    let stats = Arc::new(Stats::new());
    
    loop {
        let (socket, addr) = listener.accept().await?;
        let stats = Arc::clone(&stats);
        let config = config.clone();
        
        tokio::spawn(async move {
            stats.active_connections.fetch_add(1, Ordering::SeqCst);
            if let Err(e) = handle_connection(socket, stats.clone(), &config).await {
                error!("Erro de {}: {}", addr, e);
                stats.total_errors.fetch_add(1, Ordering::SeqCst);
            }
            stats.active_connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

async fn handle_connection(mut socket: TcpStream, stats: Arc<Stats>, config: &Config) -> Result<()> {
    let mut first_byte = [0u8; 1];
    timeout(config.connection_timeout, socket.read_exact(&mut first_byte)).await
        .map_err(|_| anyhow!("Timeout reading first byte"))??;

    match first_byte[0] {
        0x05 => {
            stats.total_socks5.fetch_add(1, Ordering::SeqCst);
            info!("SOCKS5 connection");
            handle_socks5(socket, config).await
        }
        b'G' | b'P' | b'C' | b'H' | b'D' | b'O' | b'T' => {
            let mut headers_str = String::from_utf8_lossy(&[first_byte[0]]).to_string();
            headers_str.push_str(&read_http_headers_with_timeout(&mut socket, config.connection_timeout).await?);
            let headers = HttpHeaders::new(headers_str);
            
            if headers.is_websocket() {
                stats.total_websocket.fetch_add(1, Ordering::SeqCst);
                info!("WebSocket connection");
                handle_websocket(socket, headers, config).await
            } else if headers.has_auth() {
                stats.total_security.fetch_add(1, Ordering::SeqCst);
                info!("Security connection");
                handle_security(socket, headers, config).await
            } else {
                anyhow::bail!("Unsupported HTTP")
            }
        }
        _ => anyhow::bail!("Unknown protocol"),
    }
}
MAINEOF

    # Compilar
    echo -e "${YELLOW}🛠️ Compilando VSProxy completo (pode levar alguns minutos)...${NC}"
    cargo build --release
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}❌ Erro na compilação!${NC}"
        exit 1
    fi

    # Instalar
    mkdir -p /usr/local/bin
    cp target/release/vsproxy /usr/local/bin/vsproxy-bin
    chmod +x /usr/local/bin/vsproxy-bin

    # Menu
    cat > /usr/local/bin/vsproxy << 'MENUEOF'
#!/bin/bash
echo "====================================="
echo "     VSProxy Management Menu         "
echo "====================================="
echo "1. Status do serviço"
echo "2. Reiniciar serviço"
echo "3. Parar serviço"
echo "4. Ver logs"
echo "5. Testar conexão"
echo "6. Sair"
echo "====================================="
read -p "Escolha uma opção: " option

case $option in
    1) systemctl status vsproxy --no-pager ;;
    2) systemctl restart vsproxy && echo "✅ Reiniciado" ;;
    3) systemctl stop vsproxy && echo "⏹️ Parado" ;;
    4) journalctl -u vsproxy -f ;;
    5) curl -I http://localhost:80 ;;
    6) exit ;;
    *) echo "Opção inválida" ;;
esac
MENUEOF
    chmod +x /usr/local/bin/vsproxy
}

# ============================================
# CONFIGURAR SYSTEMD
# ============================================
setup_systemd() {
    echo -e "${YELLOW}⚙️ Configurando serviço...${NC}"
    cat > /etc/systemd/system/vsproxy.service << EOF
[Unit]
Description=VSProxy Multiprotocol Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/etc/vsproxy
ExecStart=/usr/local/bin/vsproxy-bin
Restart=always
RestartSec=3
Environment=RUST_LOG=info
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

    mkdir -p /etc/vsproxy
    systemctl daemon-reload
    systemctl enable vsproxy
    systemctl restart vsproxy
}

# ============================================
# VERIFICAR
# ============================================
verify_installation() {
    sleep 2
    echo -e "${GREEN}✅ Instalação concluída!${NC}"
    echo -e "📌 Serviço: vsproxy"
    echo -e "📌 Porta: 80 (redirecionada para 8080)"
    echo -e "📌 Status: $(systemctl is-active vsproxy)"
    echo -e "\n${YELLOW}Comandos:${NC}"
    echo -e "  • Menu: vsproxy"
    echo -e "  • Logs: journalctl -u vsproxy -f"
    echo -e "  • Status: systemctl status vsproxy"
}

# ============================================
# EXECUTAR
# ============================================
main() {
    stop_conflicting_services
    install_dependencies
    install_rust
    setup_port_redirect
    compile_and_install
    setup_systemd
    verify_installation
}

main
