# ShadowPainel

Painel local, em **Rust**, para configurar e administrar servidores Minecraft no computador pessoal. A interface é Svelte e o núcleo HTTP/ciclo de vida é Rust com Axum e Tokio.

## O que já funciona

- Dashboard local servido pelo binário Rust em `http://127.0.0.1:4173`.
- Iniciar/parar o estado do servidor, uptime e métricas locais.
- Console com envio de comandos e histórico de logs.
- Tela de configurações para nome, versão, pasta e memória.
- Diagnóstico do caminho do Java via `JAVA_HOME`.
- Sem conta, sem nuvem e sem envio de dados.

> Esta versão é uma base de teste segura: o fluxo de servidor ainda simula o processo Java. O próximo passo é conectar o `tokio::process::Command` ao `server.jar` configurado e persistir os perfis em SQLite.

## Rodar para testar

Pré-requisitos: Rust 1.80+, Node.js 20+ e npm.

```bash
cd frontend
npm ci
npm run build
cd ../backend
cargo run
```

Abra `http://localhost:4173`. Para verificar o núcleo: `curl http://localhost:4173/api/health`.

## Build de produção

```bash
cd frontend && npm ci && npm run build
cd ../backend && cargo build --release
./target/release/shadowpainel
```

## Arch Linux / AUR

A receita de teste está em [`packaging/aur/PKGBUILD`](packaging/aur/PKGBUILD). Depois que o repositório estiver publicado:

```bash
git clone https://aur.archlinux.org/shadowpainel-git.git
cd shadowpainel-git
makepkg -si
```

Para um teste direto antes de enviar ao AUR:

```bash
cd packaging/aur
makepkg -f
sudo pacman -U shadowpainel-git-*.pkg.tar.zst
```

## Publicar no GitHub

```bash
git add .
git commit -m "feat: consolidar painel Rust e pacote Arch Linux"
git push origin main
```

## Licença

MIT.

## Releases multiplataforma

Ao criar uma tag `v*`, o GitHub Actions compila e publica automaticamente pacotes para:

- Linux x86_64 (`.tar.gz`)
- Windows x86_64 (`.zip`)
- macOS Intel (`.tar.gz`)
- macOS Apple Silicon/ARM64 (`.tar.gz`)

Baixe a versão desejada em [Releases](https://github.com/NoteDarck/ShadowPainel/releases), extraia o arquivo e execute o binário dentro da pasta `backend`:

```bash
cd ShadowPainel-v0.1.0-linux-x86_64/backend
./shadowpainel
```

No Windows, execute `shadowpainel.exe` dentro da pasta `backend`. O navegador deve ser aberto em `http://localhost:4173`.

## Integração e4all

O ShadowPainel inclui uma tela dedicada para o [e4all no Modrinth](https://modrinth.com/mod/e4all). O e4all é um **mod**, não um plugin Bukkit/Spigot, e oferece builds para Fabric, Forge, NeoForge e Quilt. A integração permite selecionar:

- Mod loader e versão do Minecraft;
- pasta `mods` usada pelo perfil;
- variante Android do JAR;
- suporte a contas offline/cracked;
- suporte ao Simple Voice Chat via P2P.

O painel salva o perfil e orienta a instalação do JAR correspondente do Modrinth. A configuração efetiva do mundo continua sendo feita no menu **Abrir para LAN** do Minecraft, conforme o funcionamento do próprio mod.

## Formatos de distribuição

As releases multiplataforma também geram:

- Linux: `.tar.gz`, `.AppImage` e `.deb`;
- Windows: `.zip`, executável `.exe` e instalador `.msi`;
- macOS Intel: `.tar.gz` e `.dmg`;
- macOS Apple Silicon: `.tar.gz` e `.dmg`.
