<div align="center">

<img src="https://img.shields.io/badge/version-0.8.3-8d5dff?style=flat-square&labelColor=080a10" alt="versão"/>
<img src="https://img.shields.io/badge/Rust-1.80+-dea584?style=flat-square&logo=rust&logoColor=080a10&labelColor=080a10" alt="Rust"/>
<img src="https://img.shields.io/badge/Svelte-5-ff3e00?style=flat-square&logo=svelte&logoColor=080a10&labelColor=080a10" alt="Svelte"/>
<img src="https://img.shields.io/badge/Axum-0.8-8d5dff?style=flat-square&labelColor=080a10" alt="Axum"/>
<img src="https://img.shields.io/badge/licença-MIT-59cb8a?style=flat-square&labelColor=080a10" alt="MIT"/>

# ☾ ShadowPainel

**Servidores Minecraft, sem abrir o terminal.**

Painel desktop em Rust para criar, configurar e administrar servidores Minecraft
no seu computador — com interface Svelte, núcleo Axum + Tokio e janela nativa
via WebView. Desde a série `0.6.x`, o frontend é servido **de dentro do próprio
aplicativo**, sem navegador externo.

</div>

---

## ✨ O que você pode fazer

| | Recurso | Descrição |
|---|---|---|
| 🚀 | **Criar o servidor do zero** | Baixe o `server.jar` da versão desejada e aceite a EULA direto pelo painel. |
| 🧩 | **Mods do Modrinth e CurseForge** | Busque, filtre por versão/loader e instale mods ou plugins das duas fontes. |
| 📦 | **Instalar modpacks** | Aplique modpacks completos do Modrinth ou CurseForge, com overrides e loader automáticos. |
| ☕ | **Java gerenciado** | Detecta a versão exigida pelo Minecraft/loader — incluindo Java 25 para 26.x —, baixa o runtime correto do Adoptium e usa automaticamente, sem pedir caminhos. |
| 💾 | **Backups e desfazer** | Backup automático antes de instalar conteúdo, restauração em um clique e lixeira temporária para conteúdo e backups. |
| 📦 | **Instalar em lote** | Selecione vários projetos e instale todos de uma vez com um clique. |
| 🖱️ | **Seleção por duplo clique** | Dê duplo clique no nome de um projeto ou arquivo instalado para selecioná-lo; o item fica destacado para ações em lote. |
| ♾️ | **Catálogo com scroll infinito** | Ao chegar ao fim da janela de resultados, mais mods, plugins ou datapacks são carregados automaticamente. |
| 🎯 | **Escolher a versão do mod** | Diferente da versão do Minecraft: escolha a build exata que o servidor vai rodar. |
| 🔌 | **Plugins e datapacks** | Suporte a Paper, Purpur e datapacks no mesmo catálogo. |
| 🧵 | **Instalar o Fabric** | Pronto para rodar mods e o e4all. |
| 🌐 | **Link público do e4all** | Detecte o domínio no log com status ativo/expirado, QR Code e compartilhamento direto no WhatsApp ou Discord. |
| 🩺 | **Saúde e rede** | Semáforo de Java, memória, disco, jogadores e backups, além do teste “Meus amigos conseguem entrar?” para porta, firewall, internet e UPnP. |
| 💌 | **Whitelist fácil** | Convide jogadores pelo nome; o painel resolve o UUID original ou offline e aplica a lista no servidor. |
| 🔄 | **Atualizações com rollback** | Verifica automaticamente, baixa e valida o instalador oficial por SHA-256; se precisar, baixa de novo a versão anterior. |
| 🧭 | **Erros acionáveis** | Mensagens explicam o problema e oferecem ações como tentar novamente, baixar Java, abrir a pasta ou trocar a porta. |
| 🎛️ | **Gerenciar o servidor** | Iniciar, parar, reiniciar, enviar comandos e acompanhar logs resumidos em linguagem humana, além de CPU, memória e jogadores. |
| 🖥️ | **Abrir em tela cheia** | A janela já abre maximizada, como um software dedicado. |
| 🔒 | **Execução local** | Tudo roda na sua máquina — seus dados não saem do PC. |

---

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
