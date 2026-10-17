# ShadowPainel

Painel local, em **Rust**, para configurar e administrar servidores Minecraft no computador pessoal. A interface é Svelte e o núcleo HTTP/ciclo de vida é Rust com Axum e Tokio. Desde a série `0.6.x`, o frontend é hospedado em uma WebView nativa dentro de uma janela própria do aplicativo.

## Arquitetura revisada

- `backend/`: núcleo Rust responsável pelo processo Java, console, métricas, persistência e API local.
- `desktop/`: shell Rust/WebView multiplataforma; inicia o núcleo em uma porta loopback efêmera e encerra o processo ao fechar a janela.
- `frontend/`: interface Svelte compilada como assets estáticos, sem estado de servidor próprio.
- `packaging/`: integração AUR, desktop entry, ícone e receitas de distribuição.

As configurações são gravadas localmente em `~/.shadowpainel/config.json` por substituição atômica. Isso evita deixar um JSON parcialmente escrito após queda de energia ou encerramento inesperado.

## O que já funciona

- Dashboard local exibido em uma janela desktop própria, com WebView nativa e núcleo Rust embutido.
- Iniciar/parar o estado do servidor, uptime e métricas locais.
- Console com envio de comandos e histórico de logs.
- Tela de configurações para nome, versão, pasta e memória.
- Diagnóstico do caminho do Java via `JAVA_HOME`.
- Inicia o `server.jar` real com Java, acompanha stdout/stderr e envia comandos pelo console.
- Se a porta 4173 já estiver ocupada, escolhe automaticamente a próxima porta livre até 4193. Também é possível fixar uma porta com `SHADOWPAINEL_ADDR=127.0.0.1:4174`.
- Configurações persistidas em `~/.shadowpainel/config.json`, sem conta, sem nuvem e sem envio de dados.
- Validações de configuração e e4all cobertas por testes unitários Rust.
- O botão **Reiniciar servidor** aguarda o encerramento gracioso antes de iniciar novamente.

> O painel exige um `server.jar` na pasta configurada. O servidor é executado localmente com o Java detectado em `JAVA_HOME` ou no PATH.

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

Ao criar uma tag `v*`, o GitHub Actions compila e publica automaticamente Linux x86_64 (`.tar.gz`, `.AppImage` e `.deb`), Windows x86_64 (`.zip` e `.msi`), macOS Intel (`.tar.gz` e `.dmg`) e macOS Apple Silicon/ARM64 (`.tar.gz` e `.dmg`). A release também publica `SHA256SUMS.txt` para verificação de integridade.

Baixe a versão desejada em [Releases](https://github.com/NoteDarck/ShadowPainel/releases). Ao abrir o AppImage pelo gerenciador de arquivos ou pelo atalho do sistema, o núcleo é iniciado em segundo plano e a interface abre em uma janela própria do ShadowPainel, sem navegador externo. No CachyOS/Arch, o AppImage requer os runtimes `gtk3` e `webkit2gtk-4.1`, normalmente já presentes em ambientes gráficos; o pacote AUR declara essas dependências.

## Integração e4all

O ShadowPainel inclui uma tela dedicada para o [e4all no Modrinth](https://modrinth.com/mod/e4all). O e4all é um **mod**, não um plugin Bukkit/Spigot, e oferece builds para Fabric, Forge, NeoForge e Quilt. A integração permite selecionar:

- Mod loader e versão do Minecraft;
- pasta `mods` usada pelo perfil;
- variante Android do JAR;
- suporte a contas offline/cracked;
- suporte ao Simple Voice Chat via P2P.

O painel salva o perfil e orienta a instalação do JAR correspondente do Modrinth. A configuração efetiva do mundo continua sendo feita no menu **Abrir para LAN** do Minecraft, conforme o funcionamento do próprio mod.

O e4all cria o túnel dentro do Minecraft e gera o domínio público no chat. Por isso, o ShadowPainel não tenta falsamente criar esse túnel a partir de um `server.jar` dedicado: ele permite registrar e copiar o domínio gerado pelo mod. Para usar a conexão, instale o e4all no computador host, abra o mundo no Minecraft, escolha **Abrir para LAN**, copie o domínio mostrado no chat e cole-o no campo **Domínio público e4all**.

## Formatos de distribuição

As releases multiplataforma também geram:

- Linux: `.tar.gz`, `.AppImage` e `.deb`;
- Windows: `.zip`, executável `.exe` e instalador `.msi`;
- macOS Intel: `.tar.gz` e `.dmg`;
- macOS Apple Silicon: `.tar.gz` e `.dmg`.

## Qualidade da versão 0.6.x

Cada alteração da série `0.6.x` passa por `cargo fmt`, `cargo test`, `cargo clippy -D warnings`, `cargo check` do shell desktop, `svelte-check` e build do frontend. O workflow de release também compila a WebView nativa e publica checksums SHA-256. O AppImage e o `.deb` usam a mesma arquitetura do AUR: janela desktop, núcleo separado e assets Svelte compartilhados.
