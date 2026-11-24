<div align="center">

<img src="https://img.shields.io/badge/version-0.10.7-8d5dff?style=flat-square&labelColor=080a10" alt="versão"/>
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
| 🚀 | **Criar o servidor do zero** | Baixe o `server.jar` da versão desejada ou importe um arquivo que você já tenha baixado. |
| 🧩 | **Mods do Modrinth e CurseForge** | Busque, filtre por versão/loader e instale mods ou plugins das duas fontes. |
| 📦 | **Instalar modpacks** | Aplique modpacks completos do Modrinth ou CurseForge, com overrides e loader automáticos. |
| ☕ | **Java gerenciado** | Detecta a versão exigida pelo Minecraft/loader — incluindo Java 25 para 26.x —, baixa o runtime correto do Adoptium e usa automaticamente, sem pedir caminhos. |
| 💾 | **Backups e desfazer** | Backup automático antes de instalar conteúdo, restauração em um clique e lixeira temporária para conteúdo e backups. |
| 🚚 | **Transferir servidor** | Exporte mundo, mods, plugins e configurações em um ZIP e importe em outro ShadowPainel, com o servidor já pronto para iniciar. |
| 📦 | **Instalar em lote** | Selecione vários projetos e instale todos de uma vez com um clique. |
| 🖱️ | **Seleção com um clique** | Clique em um projeto ou arquivo instalado para selecioná-lo; use o rodapé para instalar ou remover em lote. |
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
| 🧠 | **Console inteligente** | Histórico de comandos separado por servidor, setas para navegar, sugestões com Tab e filtros para jogadores, avisos e erros. |
| 📈 | **Monitoramento avançado** | Histórico visual da sessão para CPU, RAM e jogadores, uptime e espaço ocupado pelo servidor. |
| 🧪 | **Diagnóstico de conteúdo** | Sinaliza arquivos que parecem duplicar o mesmo projeto instalado e mantém a compatibilidade filtrada pelo loader escolhido. |
| 🌍 | **Mundos e dimensões** | Crie, duplique, renomeie, ative, faça backup e veja dimensões Nether/End ou customizadas detectadas em cada mundo. |
| 🖥️ | **Abrir em tela cheia** | A janela já abre maximizada, como um software dedicado. |
| 🔒 | **Execução local** | Tudo roda na sua máquina — seus dados não saem do PC. |
| 🌐 | **ShadowTunnel** | Controle túneis TCP/UDP do plugin Paper, com status, endereços públicos e conexão pelo Gateway API. |
| 🎮 | **Playit.gg** | Detecte, inicie e pare o agente Playit local e configure o túnel Minecraft Java sem abrir portas no roteador. |
| 📱 | **Cliente Android** | Acompanhe o servidor pela rede local, veja métricas, troque perfis e inicie ou pare o servidor pelo celular. |
| 🧱 | **Catálogo MCJars completo** | Crie servidores usando as opções disponíveis no MCJars, incluindo Vanilla, Paper, Arclight, Forge, NeoForge, Fabric, proxies e variantes compatíveis. |
| 🔍 | **Detecção automática de JAR** | Ao importar um `server.jar` externo, identifica o loader pelo conteúdo do arquivo — inclusive Arclight — em vez de assumir Vanilla ou Paper. |
| 🪟 | **Instalação Windows aprimorada** | O executável nativo recebe o ícone do ShadowPainel e o instalador cria atalhos funcionais no Menu Iniciar e na Área de Trabalho. |

---

## ShadowTunnel

A integração controla o plugin ShadowTunnel para Paper através do Gateway API local. O servidor Minecraft continua executando localmente; apenas o tráfego configurado pelo túnel é encaminhado.

### Instalação do plugin

1. Baixe o JAR compatível com a versão do Paper na página de releases do ShadowTunnel.
2. Copie o JAR para `plugins/` do servidor Paper.
3. Inicie o servidor uma vez, confirme que o plugin carregou e pare o servidor.
4. Edite `plugins/ShadowTunnel/config.yml`, habilite o Gateway API e defina a porta (padrão `9100`).
5. Reinicie o servidor. O painel deve alcançar `http://127.0.0.1:9100`.

### Configuração

Na seção **Configurações avançadas → Integrações → Shadow Tunnel**, informe o nome do túnel, destino local (`127.0.0.1:25565`) e tipo (`TCP` ou `UDP`). Para outro endereço do Gateway, defina `SHADOW_TUNNEL_API_URL`; para autenticação, defina `SHADOW_TUNNEL_API_TOKEN`. Não exponha a porta do Gateway publicamente.

### Uso e diagnóstico

Use **Criar túnel**, **Conectar**, **Desconectar** e **Atualizar**. Compartilhe somente o endereço TCP/UDP exibido como online. Status `requested` indica solicitação pendente; confirme o console do plugin, a porta do Gateway, o token e as regras do firewall. Teste o Gateway com `curl http://127.0.0.1:9100/api/tunnels/<nome>/status`.

A API local do painel encaminha `GET /api/shadowtunnel/status`, `POST /api/shadowtunnel/create`, `POST /api/shadowtunnel/connect` e `POST /api/shadowtunnel/disconnect` para o Gateway. Para desenvolvimento, execute `cargo fmt --all`, `cargo test` e `cargo clippy --all-targets --all-features -- -D warnings` em `backend/`; execute `npm run check` e `npm run build` em `frontend/`.

## Playit.gg

Na seção **Configurações avançadas → Integrações → Playit.gg**, o ShadowPainel verifica se o comando `playit` está instalado, permite informar um caminho alternativo e pode iniciar ou parar o agente local. O agente deve ser instalado a partir do [download oficial do Playit](https://playit.gg/download); no Linux, a documentação oficial também descreve a instalação por APT e a execução do binário.

Depois de iniciar o agente, conclua a vinculação no navegador do Playit e crie um túnel **Minecraft Java** apontando para `127.0.0.1` e a porta do servidor mostrada pelo painel. O endereço público, a conta, o segredo do agente e a configuração detalhada do túnel permanecem no Playit. O ShadowPainel controla apenas o processo local e não armazena credenciais privadas.

O agente Playit precisa permanecer na mesma máquina do servidor Minecraft. O Playit informa que seus túneis encaminham o tráfego sem exigir redirecionamento de portas no roteador; a disponibilidade depende de o agente estar conectado e do túnel estar ativo no painel do Playit. Consulte também a [documentação oficial do agente](https://github.com/playit-cloud/playit-agent/blob/master/README.md).

## Cliente Android

O cliente companion em [`mobile/`](mobile/) usa Expo e se conecta ao ShadowPainel na rede local. Para usar o celular, inicie o desktop com `SHADOWPAINEL_ADDR=0.0.0.0:4173 SHADOWPAINEL_PUBLIC_ADDR=127.0.0.1:4173 shadowpainel-desktop`, informe no aplicativo o IP do computador com a porta `4173` e mantenha os dois dispositivos na mesma rede. O cliente Android oferece status, métricas, troca de servidor, iniciar/parar e console recente; a interface completa continua disponível no desktop. Não exponha a porta diretamente à internet.

## MCJars e atualizações

Na criação ou configuração de um servidor, o painel consulta o catálogo público do [MCJars](https://mcjars.app/) e exibe os tipos de servidor disponíveis. A instalação segue as ações publicadas pela API do catálogo, incluindo downloads diretos e pacotes que precisam ser extraídos. Se a consulta estiver indisponível, a interface mantém uma lista local dos tipos conhecidos.

JARs importados também são inspecionados como arquivos ZIP. O painel reconhece marcadores de loaders como Arclight, Mohist, Forge, NeoForge, Paper, Purpur, Fabric e Quilt e salva o tipo identificado no servidor.

As atualizações do ShadowPainel validam o pacote por SHA-256 antes de instalar. No Linux, o painel usa o `.deb` somente quando `dpkg` está disponível; em distribuições Arch, como CachyOS, escolhe automaticamente o AppImage e executa com fallback para ambientes sem FUSE. O rollback continua disponível pela própria tela de atualização.

### Servidores híbridos e configurações recomendadas

Para Arclight, Magma e Mohist, o painel agora permite escolher o **loader-base** usado pelo servidor: Forge, NeoForge ou Fabric. Essa escolha é salva na configuração do servidor e é usada para filtrar mods no Modrinth/CurseForge; plugins são consultados como Paper e instalados em `plugins/`, porque os híbridos expõem compatibilidade Bukkit/Spigot/Paper. Em um servidor Magma, use NeoForge como padrão; confirme o loader do JAR antes de instalar mods.

O painel também aceita plugins para híbridos compatíveis e não libera plugins em servidores Forge, NeoForge ou Fabric puros. Isso evita o erro “Plugins precisam de um servidor Paper ou Purpur” sem prometer compatibilidade que o loader não oferece. O servidor deve estar parado antes de instalar ou atualizar conteúdo; faça backup e confira dependências como Vault, ProtocolLib ou PlaceholderAPI quando o plugin exigir.

Na aba **Configurações → Mundo e rede**, use `view-distance` entre 6 e 10 e `simulation-distance` entre 4 e 8 como ponto de partida para servidores híbridos, ajustando conforme CPU, memória e número de jogadores. Para Magma, a documentação recomenda Java 21+, `-Xms` igual a `-Xmx`, além de revisar `spigot.yml` e `bukkit.yml`; o painel já expõe as opções principais de `server.properties`. Em Paper/Purpur, configurações avançadas ficam nos arquivos YAML próprios do software e não devem ser copiadas para Forge/Fabric. Consulte as referências oficiais: [Magma — plugins](https://magmafoundation.org/docs/guides/adding-plugins), [Magma — otimização](https://magmafoundation.org/docs/guides/optimization), [Paper](https://papermc.io/software/paper), [Purpur](https://purpurmc.org/) e [Fabric — servidor](https://fabricmc.net/use/server/).

### APK Android

O workflow de release gera um APK instalável do cliente Expo em `mobile/`. Essa primeira versão é um companion: conecta-se ao ShadowPainel desktop pela rede local, mostra status, métricas e console, permite trocar o servidor ativo e iniciar ou parar a instância.

A execução local de servidores Minecraft no Android, usando PRoot ou outro runtime rootless, permanece como uma etapa separada. PRoot não é Docker real e não fornece isolamento por namespaces, cgroups ou seccomp; além disso, o Android pode encerrar processos por bateria, CPU ou memória. O APK atual não promete executar o servidor dentro do telefone.

---

## 🏗️ Arquitetura


- `backend/` — núcleo Rust responsável pelo processo Java, console, métricas, downloads, persistência e API local.
- `desktop/` — shell Rust/WebView multiplataforma; inicia o núcleo em uma porta loopback efêmera e encerra o processo ao fechar a janela.
- `frontend/` — interface Svelte compilada como assets estáticos, sem estado de servidor próprio.
- `packaging/` — integração AUR, desktop entry, ícone e receitas de distribuição.

As configurações são gravadas localmente em `~/.shadowpainel/config.json` por substituição atômica. Isso evita deixar um JSON parcialmente escrito após queda de energia ou encerramento inesperado.

Na versão `0.10.0`, a troca entre perfis preserva o console de cada servidor durante a sessão, mas o núcleo ainda executa um processo Java por vez. Assim, servidores não selecionados aparecem offline até que o usuário os inicie quando estiverem ativos; a execução simultânea de vários processos continua sendo uma evolução separada do gerenciador de processos.

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
