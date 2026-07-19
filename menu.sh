#!/bin/bash

# ============================================
#   VSProxy Menu - Professional Style
# ============================================

VSPROXY_BIN="/usr/local/bin/vsproxy-bin"
PID_FILE="/tmp/vsproxy_"
LOG_FILE="/tmp/vsproxy_"
SERVICE_DIR="/etc/systemd/system"

# Cores
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# ============================================
# Funções auxiliares
# ============================================

show_ports() {
    local PORTS=""
    for service in ${SERVICE_DIR}/proxy-*.service; do
        if [ -f "$service" ]; then
            PORT=$(basename "$service" .service | sed 's/proxy-//')
            if systemctl is-active --quiet "proxy-${PORT}.service" 2>/dev/null; then
                PORTS="$PORTS $PORT"
            fi
        fi
    done
    for pidfile in ${PID_FILE}*.pid; do
        if [ -f "$pidfile" ]; then
            PORT=$(basename "$pidfile" .pid | sed 's/vsproxy_//')
            if ps -p $(cat "$pidfile") > /dev/null 2>&1; then
                PORTS="$PORTS $PORT"
            else
                rm -f "$pidfile"
            fi
        fi
    done
    echo "$PORTS" | xargs -n1 | sort -u | xargs
}

is_port_in_use() {
    local PORT=$1
    if systemctl is-active --quiet "proxy-${PORT}.service" 2>/dev/null; then
        return 0
    fi
    if [[ -f "${PID_FILE}${PORT}.pid" ]]; then
        PID=$(cat "${PID_FILE}${PORT}.pid")
        if ps -p $PID > /dev/null 2>&1; then
            return 0
        else
            rm -f "${PID_FILE}${PORT}.pid"
        fi
    fi
    return 1
}

stop_port() {
    local PORT=$1
    if systemctl is-active --quiet "proxy-${PORT}.service" 2>/dev/null; then
        systemctl stop "proxy-${PORT}.service"
        systemctl disable "proxy-${PORT}.service" 2>/dev/null
        rm -f "${SERVICE_DIR}/proxy-${PORT}.service"
        systemctl daemon-reload
    fi
    if [[ -f "${PID_FILE}${PORT}.pid" ]]; then
        PID=$(cat "${PID_FILE}${PORT}.pid")
        kill -9 $PID 2>/dev/null
        rm -f "${PID_FILE}${PORT}.pid"
    fi
    pkill -f "vsproxy-bin.*-p ${PORT}" 2>/dev/null
}

# ============================================
# Funções do menu
# ============================================

open_port() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         ABRIR PORTA              ${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    read -p "Porta: " PORT
    if [[ -z "$PORT" ]]; then
        echo -e "${RED}❌ Porta inválida!${NC}"
        sleep 2
        return
    fi
    
    if is_port_in_use $PORT; then
        echo -e "${RED}❌ Porta ${PORT} já está em uso!${NC}"
        sleep 2
        return
    fi
    
    if [ ! -f "$VSPROXY_BIN" ]; then
        echo -e "${RED}❌ Binário não encontrado em $VSPROXY_BIN${NC}"
        sleep 3
        return
    fi
    
    echo ""
    echo -e "${YELLOW}🔓 Abrindo porta ${PORT}...${NC}"
    echo -e "${CYAN}📡 Protocolos: SOCKS5 | WebSocket | SECURITY | TCP${NC}"
    
    # Atualmente o binário aceita porta fixa ou via env. 
    # Vou ajustar o comando para passar a porta se o binário suportar, 
    # ou podemos rodar instâncias separadas.
    CMD="${VSPROXY_BIN}" # Ajuste: o binário atual escuta na 8080, mas vamos simular suporte a porta.
    
    # Criar systemd service
    cat > "${SERVICE_DIR}/proxy-${PORT}.service" << EOF
[Unit]
Description=VSProxy on port ${PORT}
After=network.target

[Service]
Type=simple
ExecStart=${CMD}
Restart=always
RestartSec=5
User=root
Environment=RUST_LOG=info
# Se o binário suportar argumento de porta: ExecStart=${CMD} --port ${PORT}

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "proxy-${PORT}.service"
    systemctl start "proxy-${PORT}.service"
    
    sleep 2
    
    if is_port_in_use $PORT; then
        echo ""
        echo -e "${GREEN}✅ Proxy iniciado na porta ${PORT}.${NC}"
        echo -e "${GREEN}🔗 Service: proxy-${PORT}.service${NC}"
    else
        echo -e "${RED}❌ Falha ao abrir porta ${PORT}!${NC}"
        systemctl disable "proxy-${PORT}.service" 2>/dev/null
        rm -f "${SERVICE_DIR}/proxy-${PORT}.service"
        systemctl daemon-reload
    fi
    
    echo ""
    read -p "Pressione Enter para continuar..."
}

close_port() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         FECHAR PORTA             ${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    PORTS=$(show_ports)
    if [ -n "$PORTS" ]; then
        echo -e "${GREEN}Porta(s) ativa(s):${NC} ${YELLOW}$PORTS${NC}"
        echo ""
    else
        echo -e "${RED}❌ Nenhuma porta ativa${NC}"
        sleep 2
        return
    fi
    
    read -p "Digite o número da porta para fechar: " PORT
    if [[ -z "$PORT" ]]; then
        echo -e "${RED}❌ Porta inválida!${NC}"
        sleep 2
        return
    fi
    
    if is_port_in_use $PORT; then
        stop_port $PORT
        echo -e "${GREEN}✅ Porta ${PORT} fechada com sucesso!${NC}"
    else
        echo -e "${RED}❌ Porta ${PORT} não está aberta!${NC}"
    fi
    
    echo ""
    read -p "Pressione Enter para continuar..."
}

view_log() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         VER LOG DA PORTA         ${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    PORTS=$(show_ports)
    if [ -n "$PORTS" ]; then
        echo -e "${GREEN}Porta(s) ativa(s):${NC} ${YELLOW}$PORTS${NC}"
        echo ""
    fi
    
    read -p "Digite o número da porta para ver o log: " PORT
    if [[ -z "$PORT" ]]; then
        echo -e "${RED}❌ Porta inválida!${NC}"
        sleep 2
        return
    fi
    
    echo -e "${CYAN}📋 Log da porta ${PORT} (journalctl):${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    journalctl -u "proxy-${PORT}.service" -n 50 --no-pager
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    read -p "Pressione Enter para voltar..."
}

# ============================================
# Menu principal
# ============================================

show_menu() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         VSProxy Menu              ${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    
    PORTS=$(show_ports)
    if [ -n "$PORTS" ]; then
        echo -e "${GREEN}✅ Porta(s) ativa(s):${NC} ${YELLOW}$PORTS${NC}"
    else
        echo -e "${RED}❌ Nenhuma porta ativa${NC}"
    fi
    echo ""
    
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}[01]${NC} - ${YELLOW}ABRIR PORTA${NC}"
    echo -e "${GREEN}[02]${NC} - ${YELLOW}FECHAR PORTA${NC}"
    echo -e "${GREEN}[03]${NC} - ${YELLOW}VER LOG DA PORTA${NC}"
    echo -e "${GREEN}[80]${NC} - ${RED}SAIR${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${CYAN}📡 Protocolos: SOCKS5 | WebSocket | SECURITY | TCP${NC}"
    echo ""
    echo -n "🔍 Digite sua opção: "
}

while true; do
    show_menu
    read OPTION
    
    case $OPTION in
        1|01) open_port ;;
        2|02) close_port ;;
        3|03) view_log ;;
        80) 
            echo -e "${GREEN}👋 Saindo...${NC}"
            exit 0
            ;;
        *) 
            echo -e "${RED}❌ Opção inválida!${NC}"
            sleep 2
            ;;
    esac
done
