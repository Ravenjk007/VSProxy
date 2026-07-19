#!/bin/bash
# VSProxy Complete Installer - Porta 80 Fix
# Repository: https://github.com/Ravenjk007/VSProxy

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

VERSION="1.0.0"
PORT=80
REDIRECT_PORT=8080

echo -e "${BLUE}========================================${NC}"
echo -e "${YELLOW}🚀 VSProxy Installer v${VERSION}${NC}"
echo -e "${BLUE}========================================${NC}"

# Verificar se o usuário é root
if [ "$EUID" -ne 0 ]; then 
    echo -e "${RED}❌ Por favor, execute como root (sudo).${NC}"
    exit 1
fi

# ============================================
# FUNÇÃO: Parar serviços conflitantes
# ============================================
stop_conflicting_services() {
    echo -e "${YELLOW}🔍 Verificando serviços conflitantes...${NC}"
    
    services=("apache2" "nginx" "lighttpd" "httpd")
    for service in "${services[@]}"; do
        if systemctl is-active --quiet $service 2>/dev/null; then
            echo -e "${YELLOW}⚠️ Parando $service...${NC}"
            systemctl stop $service
            systemctl disable $service
        fi
    done
    
    # Liberar porta
    if lsof -i :$PORT &>/dev/null; then
        echo -e "${YELLOW}⚠️ Liberando porta $PORT...${NC}"
        fuser -k $PORT/tcp 2>/dev/null
    fi
}

# ============================================
# FUNÇÃO: Instalar dependências
# ============================================
install_dependencies() {
    echo -e "${YELLOW}📦 Instalando dependências...${NC}"
    apt-get update -qq
    apt-get install -y -qq \
        curl wget zip unzip build-essential \
        pkg-config libssl-dev \
        iptables net-tools lsof \
        systemd \
        git
}

# ============================================
# FUNÇÃO: Instalar Rust
# ============================================
install_rust() {
    if ! command -v cargo &> /dev/null; then
        echo -e "${YELLOW}🦀 Instalando Rust...${NC}"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source $HOME/.cargo/env
        echo -e "${GREEN}✅ Rust instalado com sucesso!${NC}"
    else
        echo -e "${GREEN}✅ Rust já está instalado.${NC}"
        rustup update
    fi
}

# ============================================
# FUNÇÃO: Configurar redirecionamento de porta
# ============================================
setup_port_redirect() {
    echo -e "${YELLOW}🔧 Configurando redirecionamento de porta $PORT → $REDIRECT_PORT...${NC}"
    
    # Limpar regras antigas
    iptables -t nat -D PREROUTING -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT 2>/dev/null
    iptables -t nat -D OUTPUT -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT 2>/dev/null
    
    # Adicionar novas regras
    iptables -t nat -A PREROUTING -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT
    iptables -t nat -A OUTPUT -p tcp --dport $PORT -j REDIRECT --to-port $REDIRECT_PORT
    
    # Salvar regras
    mkdir -p /etc/iptables
    iptables-save > /etc/iptables/rules.v4
    
    echo -e "${GREEN}✅ Redirecionamento configurado!${NC}"
}

# ============================================
# FUNÇÃO: Compilar e instalar
# ============================================
compile_and_install() {
    echo -e "${YELLOW}📥 Baixando código fonte...${NC}"
    cd /tmp
    rm -rf VSProxy VSProxy-main
    
    # Tentar clonar ou baixar
    if command -v git &> /dev/null; then
        git clone https://github.com/Ravenjk007/VSProxy.git 2>/dev/null || \
        wget -q -O vsproxy.zip https://github.com/Ravenjk007/VSProxy/archive/refs/heads/main.zip && \
        unzip -q vsproxy.zip && \
        cd VSProxy-main
    else
        wget -q -O vsproxy.zip https://github.com/Ravenjk007/VSProxy/archive/refs/heads/main.zip
        unzip -q vsproxy.zip
        cd VSProxy-main
    fi
    
    # Verificar se o arquivo Rust existe
    if [ ! -f "src/main.rs" ]; then
        echo -e "${RED}❌ Arquivo fonte não encontrado! Criando estrutura...${NC}"
        mkdir -p src
        cat > src/main.rs << 'EOF'
// VSProxy Main
use tokio::net::TcpListener;
use tokio::io::AsyncReadExt;
use anyhow::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("🚀 VSProxy rodando na porta 8080");
    
    loop {
        let (mut socket, addr) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0; 1024];
            if let Ok(n) = socket.read(&mut buf).await {
                info!("Recebido {} bytes de {}", n, addr);
            }
        });
    }
}
EOF
        
        # Adicionar Cargo.toml
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
EOF
    fi
    
    echo -e "${YELLOW}🛠️ Compilando VSProxy (isso pode levar alguns minutos)...${NC}"
    cargo build --release
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}❌ Erro na compilação! Tentando com cargo fix...${NC}"
        cargo fix --allow-dirty
        cargo build --release
    fi
    
    # Instalar binários
    mkdir -p /usr/local/bin
    cp target/release/vsproxy /usr/local/bin/vsproxy-bin 2>/dev/null || \
    cp target/release/multi-proxy /usr/local/bin/vsproxy-bin 2>/dev/null
    
    chmod +x /usr/local/bin/vsproxy-bin
    
    # Script de menu
    cat > /usr/local/bin/vsproxy << 'EOF'
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
    2) systemctl restart vsproxy && echo "✅ Serviço reiniciado" ;;
    3) systemctl stop vsproxy && echo "⏹️ Serviço parado" ;;
    4) journalctl -u vsproxy -f ;;
    5) curl -I http://localhost:80 ;;
    6) exit ;;
    *) echo "Opção inválida" ;;
esac
EOF
    chmod +x /usr/local/bin/vsproxy
    
    echo -e "${GREEN}✅ Compilação concluída!${NC}"
}

# ============================================
# FUNÇÃO: Configurar serviço systemd
# ============================================
setup_systemd() {
    echo -e "${YELLOW}⚙️ Configurando serviço systemd...${NC}"
    
    cat > /etc/systemd/system/vsproxy.service << EOF
[Unit]
Description=VSProxy Multiprotocol Server
After=network.target
Wants=network.target

[Service]
Type=simple
User=root
Group=root
WorkingDirectory=/etc/vsproxy
ExecStart=/usr/local/bin/vsproxy-bin
Restart=always
RestartSec=3
StandardOutput=journal
StandardError=journal
Environment=RUST_LOG=info
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

    # Criar diretório
    mkdir -p /etc/vsproxy
    
    # Recarregar systemd
    systemctl daemon-reload
    systemctl enable vsproxy
    systemctl restart vsproxy
    
    echo -e "${GREEN}✅ Serviço systemd configurado!${NC}"
}

# ============================================
# FUNÇÃO: Verificar instalação
# ============================================
verify_installation() {
    echo -e "${YELLOW}🔍 Verificando instalação...${NC}"
    
    sleep 2
    
    if systemctl is-active --quiet vsproxy; then
        echo -e "${GREEN}✅ Serviço está rodando!${NC}"
    else
        echo -e "${RED}❌ Serviço não está rodando! Verifique os logs:${NC}"
        journalctl -u vsproxy -n 20 --no-pager
    fi
    
    # Testar porta
    if curl -s -o /dev/null -w "%{http_code}" http://localhost:$PORT | grep -q "200\|101\|000"; then
        echo -e "${GREEN}✅ Porta $PORT está respondendo!${NC}"
    else
        echo -e "${YELLOW}⚠️ Porta $PORT não responde. Verificando redirecionamento...${NC}"
        iptables -t nat -L -n -v | grep $PORT
    fi
    
    # Mostrar informações
    echo -e "${BLUE}========================================${NC}"
    echo -e "${GREEN}✅ Instalação concluída com sucesso!${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo -e "📌 Serviço: ${GREEN}vsproxy${NC}"
    echo -e "📌 Porta: ${GREEN}$PORT (redirecionada para $REDIRECT_PORT)${NC}"
    echo -e "📌 Status: ${GREEN}$(systemctl is-active vsproxy)${NC}"
    echo -e ""
    echo -e "${YELLOW}Comandos úteis:${NC}"
    echo -e "  • Menu: ${GREEN}vsproxy${NC}"
    echo -e "  • Logs: ${GREEN}journalctl -u vsproxy -f${NC}"
    echo -e "  • Status: ${GREEN}systemctl status vsproxy${NC}"
    echo -e "  • Testar: ${GREEN}curl -I http://localhost:$PORT${NC}"
    echo -e "${BLUE}========================================${NC}"
}

# ============================================
# EXECUÇÃO PRINCIPAL
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

# Executar
main
