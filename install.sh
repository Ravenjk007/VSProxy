#!/bin/bash
# VSProxy Installer

# ---->>>> Configure aqui o seu repositório
REPO_URL="https://github.com/Ravenjk007/VSProxy.git"
REPO_BRANCH="main"
CMD_NAME="vsproxy"

TOTAL_STEPS=9
CURRENT_STEP=0

show_progress() {
    PERCENT=$((CURRENT_STEP * 100 / TOTAL_STEPS))
    echo "Progresso: [${PERCENT}%] - $1"
}

error_exit() {
    echo -e "\nErro: $1"
    exit 1
}

increment_step() {
    CURRENT_STEP=$((CURRENT_STEP + 1))
}

if [ "$EUID" -ne 0 ]; then
    error_exit "EXECUTE COMO ROOT"
else
    clear
    show_progress "Atualizando repositorios..."
    export DEBIAN_FRONTEND=noninteractive
    apt update -y > /dev/null 2>&1 || error_exit "Falha ao atualizar os repositorios"
    increment_step

    # ---->>>> Verificação do sistema
    show_progress "Verificando o sistema..."
    if ! command -v lsb_release &> /dev/null; then
        apt install lsb-release -y > /dev/null 2>&1 || error_exit "Falha ao instalar lsb-release"
    fi
    increment_step

    # ---->>>> Verificação do sistema
    OS_NAME=$(lsb_release -is)
    VERSION=$(lsb_release -rs)

    case $OS_NAME in
        Ubuntu)
            case $VERSION in
                24.*|22.*|20.*|18.*)
                    show_progress "Sistema Ubuntu suportado, continuando..."
                    ;;
                *)
                    error_exit "Versão do Ubuntu não suportada. Use 18, 20, 22 ou 24."
                    ;;
            esac
            ;;
        Debian)
            case $VERSION in
                12*|11*|10*|9*)
                    show_progress "Sistema Debian suportado, continuando..."
                    ;;
                *)
                    error_exit "Versão do Debian não suportada. Use 9, 10, 11 ou 12."
                    ;;
            esac
            ;;
        *)
            error_exit "Sistema não suportado. Use Ubuntu ou Debian."
            ;;
    esac
    increment_step

    # ---->>>> Instalação de pacotes requisitos e atualização do sistema
    show_progress "Atualizando o sistema..."
    apt upgrade -y > /dev/null 2>&1 || error_exit "Falha ao atualizar o sistema"
    apt-get install curl build-essential git -y > /dev/null 2>&1 || error_exit "Falha ao instalar pacotes"
    increment_step

    # ---->>>> Criando o diretório do script
    show_progress "Criando diretorio /opt/vsproxy..."
    mkdir -p /opt/vsproxy > /dev/null 2>&1
    increment_step

    # ---->>>> Instalar rust
    show_progress "Instalando Rust..."
    if ! command -v rustc &> /dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y > /dev/null 2>&1 || error_exit "Falha ao instalar Rust"
        source "$HOME/.cargo/env"
    fi
    increment_step

    # ---->>>> Instalar o VSProxy
    show_progress "Compilando VSProxy, isso pode levar algum tempo dependendo da maquina..."

    if [ -d "/root/VSProxy" ]; then
        rm -rf /root/VSProxy
    fi

    git clone --branch "$REPO_BRANCH" "$REPO_URL" /root/VSProxy > /dev/null 2>&1 || error_exit "Falha ao clonar VSProxy"

    if [ -f /root/VSProxy/menu.sh ]; then
        mv /root/VSProxy/menu.sh /opt/vsproxy/menu
    fi

    cd /root/VSProxy || error_exit "Diretório do VSProxy não encontrado"
    cargo build --release --jobs "$(nproc)" > /dev/null 2>&1 || error_exit "Falha ao compilar VSProxy"
    mv ./target/release/vsproxy /opt/vsproxy/proxy || error_exit "Binário compilado não encontrado"
    increment_step

    # ---->>>> Configuração de permissões
    show_progress "Configurando permissões..."
    chmod +x /opt/vsproxy/proxy
    [ -f /opt/vsproxy/menu ] && chmod +x /opt/vsproxy/menu
    if [ -f /opt/vsproxy/menu ]; then
        ln -sf /opt/vsproxy/menu /usr/local/bin/"$CMD_NAME"
    else
        ln -sf /opt/vsproxy/proxy /usr/local/bin/"$CMD_NAME"
    fi
    increment_step

    # ---->>>> Limpeza
    show_progress "Limpando diretórios temporários..."
    cd /root/
    rm -rf /root/VSProxy/
    increment_step

    # ---->>>> Instalação finalizada :)
    echo "Instalação concluída com sucesso. Digite '$CMD_NAME' para acessar o menu."
fi
