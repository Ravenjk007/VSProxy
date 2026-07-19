#!/bin/bash

# ============================================
# BSProxy Menu - Free
# ============================================

BSPROXY="/opt/bsproxy/proxy"
PID_FILE="/tmp/bsproxy_"
LOG_FILE="/tmp/bsproxy_"
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
    # Também verifica PIDs antigos
    for pidfile in ${PID_FILE}*.pid; do
        if [ -f "$pidfile" ]; then
            PORT=$(basename "$pidfile" .pid | sed 's/bsproxy_//')
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
    # Verifica systemd
    if systemctl is-active --quiet "proxy-${PORT}.service" 2>/dev/null; then
        return 0
    fi
    # Verifica PID
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
    # Para systemd
    if systemctl is-active --quiet "proxy-${PORT}.service" 2>/dev/null; then
        systemctl stop "proxy-${PORT}.service"
        systemctl disable "proxy-${PORT}.service" 2>/dev/null
        rm -f "${SERVICE_DIR}/proxy-${PORT}.service"
        systemctl daemon-reload
    fi
    # Mata PID
    if [[ -f "${PID_FILE}${PORT}.pid" ]]; then
        PID=$(cat "${PID_FILE}${PORT}.pid")
        kill -9 $PID 2>/dev/null
        rm -f "${PID_FILE}${PORT}.pid"
    fi
    # Mata processos diretos
    pkill -f "bsproxy -p ${PORT}" 2>/dev/null
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
    
    if [ ! -f "$BSPROXY" ]; then
        echo -e "${RED}❌ BSProxy não encontrado em $BSPROXY${NC}"
        sleep 3
        return
    fi
    
    # Perguntas estilo Dtunnel
    echo ""
    read -p "Deseja habilitar SSL? (s/n) [n]: " SSL_ENABLE
    SSL_ENABLE=${SSL_ENABLE:-n}
    
    echo ""
    read -p "Resposta HTTP padrão [WebSocket]: " HTTP_MODE
    HTTP_MODE=${HTTP_MODE:-WebSocket}
    
    echo ""
    read -p "Habilitar modo somente SSH? (s/n) [n]: " SSH_ONLY
    SSH_ONLY=${SSH_ONLY:-n}
    
    echo ""
    echo -e "${YELLOW}🔓 Abrindo porta ${PORT}...${NC}"
    echo -e "${CYAN}📡 Protocolos: SOCKS5 | TLS | WebSocket | SECURITY | TCP${NC}"
    echo -e "${CYAN}🔒 SSL: ${SSL_ENABLE} | HTTP: ${HTTP_MODE} | SSH: ${SSH_ONLY}${NC}"
    
    # Construir comando
    CMD="${BSPROXY} -p ${PORT}"
    
    if [[ "$SSL_ENABLE" == "s" ]] || [[ "$SSL_ENABLE" == "S" ]]; then
        CMD="${CMD} --ssl"
    fi
    
    # Criar systemd service
    cat > "${SERVICE_DIR}/proxy-${PORT}.service" << EOF
[Unit]
Description=BSProxy on port ${PORT}
After=network.target

[Service]
Type=simple
ExecStart=${CMD}
Restart=always
RestartSec=5
User=root

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "proxy-${PORT}.service"
    systemctl start "proxy-${PORT}.service"
    
    # Também inicia com nohup (fallback)
    nohup ${CMD} > "${LOG_FILE}${PORT}.log" 2>&1 &
    echo $! > "${PID_FILE}${PORT}.pid"
    
    sleep 2
    
    if is_port_in_use $PORT; then
        echo ""
        echo -e "${GREEN}✅ Proxy iniciado na porta ${PORT}.${NC}"
        echo -e "${GREEN}📋 Log: ${LOG_FILE}${PORT}.log${NC}"
        echo -e "${GREEN}🔗 Service: proxy-${PORT}.service${NC}"
        echo ""
        echo -e "${CYAN}🧪 Testes:${NC}"
        echo -e "   ${YELLOW}SOCKS5:${NC} curl --socks5 localhost:${PORT} http://example.com"
        echo -e "   ${YELLOW}WebSocket:${NC} wscat -c ws://localhost:${PORT}"
        echo -e "   ${YELLOW}TLS:${NC} openssl s_client -connect localhost:${PORT}"
        echo -e "   ${YELLOW}HTTP:${NC} curl http://localhost:${PORT}"
    else
        echo -e "${RED}❌ Falha ao abrir porta ${PORT}!${NC}"
        rm -f "${PID_FILE}${PORT}.pid"
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

restart_port() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         REINICIAR PORTA          ${NC}"
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
    
    read -p "Digite o número da porta para reiniciar: " PORT
    if [[ -z "$PORT" ]]; then
        echo -e "${RED}❌ Porta inválida!${NC}"
        sleep 2
        return
    fi
    
    if is_port_in_use $PORT; then
        echo -e "${YELLOW}🔄 Reiniciando porta ${PORT}...${NC}"
        stop_port $PORT
        sleep 2
        open_port_restart $PORT
    else
        echo -e "${RED}❌ Porta ${PORT} não está aberta!${NC}"
        sleep 2
    fi
}

open_port_restart() {
    local PORT=$1
    
    if [ ! -f "$BSPROXY" ]; then
        echo -e "${RED}❌ BSProxy não encontrado!${NC}"
        return
    fi
    
    CMD="${BSPROXY} -p ${PORT}"
    
    # Criar systemd service
    cat > "${SERVICE_DIR}/proxy-${PORT}.service" << EOF
[Unit]
Description=BSProxy on port ${PORT}
After=network.target

[Service]
Type=simple
ExecStart=${CMD}
Restart=always
RestartSec=5
User=root

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "proxy-${PORT}.service"
    systemctl start "proxy-${PORT}.service"
    
    nohup ${CMD} > "${LOG_FILE}${PORT}.log" 2>&1 &
    echo $! > "${PID_FILE}${PORT}.pid"
    
    sleep 2
    
    if is_port_in_use $PORT; then
        echo -e "${GREEN}✅ Porta ${PORT} reiniciada!${NC}"
    else
        echo -e "${RED}❌ Falha ao reiniciar porta ${PORT}!${NC}"
    fi
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
    
    LOG_FILE="/tmp/bsproxy_${PORT}.log"
    
    if [ -f "$LOG_FILE" ]; then
        echo -e "${CYAN}📋 Log da porta ${PORT}:${NC}"
        echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        tail -50 "$LOG_FILE"
        echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        echo ""
        echo -e "${YELLOW}Dicas:${NC}"
        echo "  - Pressione 'q' para sair do tail"
        echo "  - Use: tail -f ${LOG_FILE} para acompanhar em tempo real"
        echo ""
        read -p "Pressione Enter para voltar..."
    else
        echo -e "${RED}❌ Log da porta ${PORT} não encontrado!${NC}"
        sleep 2
    fi
}

# ============================================
# Menu principal
# ============================================

show_menu() {
    clear
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}         BSProxy Menu              ${NC}"
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
    echo -e "${GREEN}[03]${NC} - ${YELLOW}REINICIAR PORTA${NC}"
    echo -e "${GREEN}[04]${NC} - ${YELLOW}VER LOG DA PORTA${NC}"
    echo -e "${GREEN}[80]${NC} - ${RED}SAIR${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "${CYAN}📡 Protocolos: SOCKS5 | TLS | WebSocket | SECURITY | TCP${NC}"
    echo -e "${CYAN}📋 Status: $(systemctl list-units --type=service --all | grep -c proxy-) serviços systemd${NC}"
    echo ""
    echo -n "🔍 Digite sua opção: "
}

# ============================================
# Loop principal
# ============================================

while true; do
    show_menu
    read OPTION
    
    case $OPTION in
        1|01) open_port ;;
        2|02) close_port ;;
        3|03) restart_port ;;
        4|04) view_log ;;
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
