# ShadowPainel

Painel **desktop** em Rust para criar, configurar e administrar servidores Minecraft no seu computador pessoal. A interface é Svelte, o núcleo é Rust (Axum + Tokio) e a janela é nativa via WebView. Desde a série `0.6.x`, o frontend é hospedado dentro do próprio aplicativo — **sem navegador externo**.

## ✨ O que você pode fazer com o ShadowPainel

- **Criar o servidor do zero** — baixe o `server.jar` da versão desejada e aceite a EULA direto pelo painel.
- **Instalar mods do Modrinth** — busque e instale mods na pasta `mods` do servidor.
- **Instalar modpacks** — baixe modpacks `.mrpack` e prepare o servidor para receber os mods.
- **Instalar o Fabric** — para rodar mods e o e4all.
- **Configurar o e4all** — registre o domínio público gerado pelo mod e compartilhe com os jogadores.
- **Gerenciar o servidor** — iniciar, parar, reiniciar, enviar comandos e acompanhar logs e métricas (CPU, memória, jogadores).
- **Abrir em tela cheia** — a janela já abre maximizada para uma experiência de software dedicado.

## 🏗️ Arquitetura

- `backend/` — núcleo Rust responsável pelo processo Java, console, métricas, downloads, persistência e API local.
- `desktop/` — shell Rust/WebView multiplataforma; inicia o núcleo em uma porta loopback efêmera e encerra o processo ao fechar a janela.
- `frontend/` — interface Svelte compilada como assets estáticos, sem estado de servidor próprio.
- `packaging/` — integração AUR, desktop entry, ícone e receitas de distribuição.

As configurações são gravadas localmente em `~/.shadowpainel/config.json` por substituição atômica. Isso evita deixar um JSON parcialmente escrito após queda de energia ou encerramento inesperado.

## 🚀 Rodar para testar

Pré-requisitos: Rust 1.80+, Node.js 20+ e npm.

```bash
# 1. Compilar o frontend
cd frontend
npm ci
npm run build

# 2. Compilar o backend
cd ../backend
cargo build --release

# 3. Compilar o desktop
cd ../desktop
cargo build --release

# 4. Executar
./target/release/shadowpainel-desktop