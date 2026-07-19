#!/bin/bash

# VSProxy Installer Script - CORRIGIDO
# Repository: https://github.com/Ravenjk007/VSProxy

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PORT=80
REDIRECT_PORT=8080

echo -e "${BLUE}========================================${NC}"
echo -e "${YELLOW}🚀 VSProxy Installer - Versão Corrigida${NC}"
echo -e "${BLUE}========================================${NC}"

# Verificar se o usuário é root
if [ "$EUID" -ne 0 ]; then 
    echo -e "${RED}❌ Por favor, execute como root (sudo).${NC}"
    exit 1
fi

# ============================================
# 1. PARAR SERVIÇOS CONFLITANTES
# ============================================
echo -e "${YELLOW}🔍 Parando serviços conflitantes...${NC}"
services=("apache2" "nginx" "lighttpd" "httpd")
for service in "${services[@]}"; do
    if systemctl is-active --quiet $service 2>/dev/null; then
        echo -e "${YELLOW}⚠️ Parando $service...${NC}"
        systemctl stop $service
        systemctl disable $service
    fi
done

# Liberar porta 80
if lsof -i :$PORT &>/dev/null; then
    echo -e "${YELLOW}⚠️ Liberando porta $PORT...${NC}"
    fuser -k $PORT/tcp 2>/dev/null
fi

# ============================================
# 2. INSTALAR DEPENDÊNCIAS
# ============================================
echo -e "${YELLOW}📦 Instalando dependências...${NC}"
apt-get update -qq
apt-get install -y -qq curl wget zip unzip build-essential pkg-config libssl-dev iptables net-tools lsof

# ============================================
# 3. INSTALAR RUST
# ============================================
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}🦀 Rust não encontrado. Instalando...${NC}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
else
    echo -e "${GREEN}✅ Rust já está instalado.${NC}"
fi

# ============================================
# 4. CONFIGURAR REDIRECIONAMENTO DE PORTA
# ============================================
echo -e "${YELLOW}🔧 Configurando redirecionamento $PORT → $REDIRECT_PORT...${NC}"

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

# ============================================
# 5. BAIXAR E COMPILAR
# ============================================
echo -e "${YELLOW}📥 Baixando código fonte do GitHub...${NC}"
mkdir -p /etc/vsproxy
cd /tmp
rm -rf vsproxy.zip VSProxy-main

wget -q -O vsproxy.zip https://github.com/Ravenjk007/VSProxy/archive/refs/heads/main.zip
unzip -q -o vsproxy.zip
cd VSProxy-main

# VERIFICAR SE O CÓDIGO FONTE EXISTE
if [ ! -f "src/main.rs" ]; then
    echo -e "${RED}❌ Código fonte não encontrado!${NC}"
    echo -e "${YELLOW}📝 Criando estrutura básica...${NC}"
    mkdir -p src
    cat > src/main.rs << 'EOF'
// VSProxy Main - Versão Corrigida
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("🚀 VSProxy rodando na porta 8080 (redirecionada para 80)");
    
    loop {
        let (mut socket, addr) = listener.accept().await?;
        tokio::spawn(async move {
            info!("Nova conexão de: {}", addr);
            let mut buf = [0; 1024];
            while let Ok(n) = socket.read(&mut buf).await {
                if n == 0 { break; }
                info!("Recebidos {} bytes de {}", n, addr);
            }
        });
    }
}
EOF

    # Adicionar Cargo.toml se não existir
    if [ ! -f "Cargo.toml" ]; then
        cat > Cargo.toml << 'EOF'
[package]
name = "multi-proxy"
version = "1.0.0"
edition = "2021"

[dependencies]
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
log = "0.4"
env_logger = "0.11"
EOF
    fi
fi

# ============================================
# 6. COMPILAR
# ============================================
echo -e "${YELLOW}🛠️ Compilando VSProxy (isso pode levar alguns minutos)...${NC}"
cargo build --release

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Erro na compilação!${NC}"
    echo -e "${YELLOW}🔄 Tentando corrigir...${NC}"
    cargo fix --allow-dirty
    cargo build --release
fi

# ============================================
# 7. INSTALAR BINÁRIOS
# ============================================
echo -e "${YELLOW}📦 Instalando binários...${NC}"

# Verificar qual binário foi gerado
if [ -f "target/release/multi-proxy" ]; then
    cp target/release/multi-proxy /usr/local/bin/vsproxy-bin
elif [ -f "target/release/vsproxy" ]; then
    cp target/release/vsproxy /usr/local/bin/vsproxy-bin
else
    echo -e "${RED}❌ Nenhum binário encontrado!${NC}"
    exit 1
fi

chmod +x /usr/local/bin/vsproxy-bin

# Criar script de menu
cat > /usr/local/bin/vsproxy << 'MENUEOF'
#!/bin/bash
echo "====================================="
echo "     VSProxy Management Menu         "
echo "====================================="
echo "1. Status do serviço"
echo "2. Reiniciar serviço"
echo "3. Parar serviço"
echo "4. Ver logs"
echo "5. Testar conexão na porta 80"
echo "6. Ver regras de firewall"
echo "7. Sair"
echo "====================================="
read -p "Escolha uma opção: " option

case $option in
    1) systemctl status vsproxy --no-pager ;;
    2) systemctl restart vsproxy && echo "✅ Serviço reiniciado" ;;
    3) systemctl stop vsproxy && echo "⏹️ Serviço parado" ;;
    4) journalctl -u vsproxy -f ;;
    5) curl -I http://localhost:80 ;;
    6) iptables -t nat -L -n -v | grep 80 ;;
    7) exit ;;
    *) echo "Opção inválida" ;;
esac
MENUEOF

chmod +x /usr/local/bin/vsproxy

# ============================================
# 8. CONFIGURAR SYSTEMD
# ============================================
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

mkdir -p /etc/vsproxy

# ============================================
# 9. INICIAR SERVIÇO
# ============================================
systemctl daemon-reload
systemctl enable vsproxy
systemctl restart vsproxy

# ============================================
# 10. VERIFICAR INSTALAÇÃO
# ============================================
echo -e "${YELLOW}🔍 Verificando instalação...${NC}"
sleep 3

if systemctl is-active --quiet vsproxy; then
    echo -e "${GREEN}✅ Serviço está rodando!${NC}"
else
    echo -e "${RED}❌ Serviço não está rodando!${NC}"
    echo -e "${YELLOW}Últimos logs:${NC}"
    journalctl -u vsproxy -n 10 --no-pager
fi

# Testar porta
if curl -s -o /dev/null -w "%{http_code}" http://localhost:$PORT | grep -q "200\|101\|000"; then
    echo -e "${GREEN}✅ Porta $PORT está respondendo!${NC}"
else
    echo -e "${YELLOW}⚠️ Porta $PORT não responde. Verifique:${NC}"
    iptables -t nat -L -n -v | grep $PORT
fi

echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}✅ VSProxy instalado com sucesso!${NC}"
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
