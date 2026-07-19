#!/bin/bash

# VSProxy Installer Script
# Repository: https://github.com/Ravenjk007/VSProxy

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}🚀 Iniciando instalação do VSProxy...${NC}"

# Verificar se o usuário é root
if [ "$EUID" -ne 0 ]; then 
  echo -e "${RED}Por favor, execute como root (sudo).${NC}"
  exit 1
fi

# Instalar dependências básicas
echo -e "${YELLOW}📦 Instalando dependências...${NC}"
apt-get update && apt-get install -y curl wget zip unzip build-essential

# Instalar Rust se não existir
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}🦀 Rust não encontrado. Instalando...${NC}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
else
    echo -e "${GREEN}✅ Rust já está instalado.${NC}"
fi

# Criar diretório do projeto
mkdir -p /etc/vsproxy
cd /tmp

# Baixar o código fonte do repositório
echo -e "${YELLOW}📥 Baixando código fonte do GitHub...${NC}"
wget -O vsproxy.zip https://github.com/Ravenjk007/VSProxy/archive/refs/heads/main.zip
unzip -o vsproxy.zip
cd VSProxy-main

# Compilar o projeto
echo -e "${YELLOW}🛠️ Compilando VSProxy (isso pode levar alguns minutos)...${NC}"
cargo build --release

# Mover binário para o sistema
cp target/release/multi-proxy /usr/local/bin/vsproxy
chmod +x /usr/local/bin/vsproxy

# Criar arquivo de serviço systemd
echo -e "${YELLOW}⚙️ Configurando serviço systemd...${NC}"
cat <<EOF > /etc/systemd/system/vsproxy.service
[Unit]
Description=VSProxy Multiprotocol Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/etc/vsproxy
ExecStart=/usr/local/bin/vsproxy
Restart=always
RestartSec=3
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# Recarregar systemd e iniciar serviço
systemctl daemon-reload
systemctl enable vsproxy
systemctl start vsproxy

echo -e "${GREEN}✅ VSProxy instalado e rodando com sucesso!${NC}"
echo -e "${YELLOW}Estatísticas: journalctl -u vsproxy -f${NC}"
echo -e "${YELLOW}Porta padrão: 8080${NC}"
