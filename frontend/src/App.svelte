<script lang="ts">
  import { onMount } from 'svelte';

  const APP_VERSION = '0.7.1';

  // ── Tipos ──
  type Log = { time:string; text:string; kind:string };
  type Version = { id:string; kind:string; release_time:string };
  type ContentItem = { filename:string; size:number; kind:string };
  type ContentResult = {
    project_id:string; title:string; description:string;
    icon_url?:string; downloads:number;
  };
  type E4all = {
    loader:string; minecraft_version:string; android_variant:boolean;
    offline_accounts:boolean; voice_chat:boolean; install_path:string;
    public_address:string;
  };
  type Server = {
    status:string; address:string; players:number; memory_mb:number;
    memory_limit_gb:number; cpu_percent:number; uptime_seconds:number;
    server_name:string; version:string; server_dir:string; java_path:string;
    java_detected:boolean; installed:boolean; e4all:E4all; logs:Log[];
  };

  // ── Estado principal ──
  let server:Server = {
    status:'offline', address:'localhost:25565', players:0, memory_mb:0,
    memory_limit_gb:4, cpu_percent:0, uptime_seconds:0,
    server_name:'Meu servidor', version:'',
    server_dir:'~/Minecraft/MeuServidor', java_path:'java',
    java_detected:false, installed:false,
    e4all:{
      loader:'fabric', minecraft_version:'', android_variant:false,
      offline_accounts:false, voice_chat:true,
      install_path:'~/Minecraft/MeuServidor/mods', public_address:''
    },
    logs:[]
  };
  let versions:Version[] = [];
  let activeTab:'overview'|'content'|'config'|'setup' = 'overview';

  // ── Aba de conteúdo ──
  let contentKind = 'mods';
  let contentLoader = 'fabric';
  let contentVersion = '';
  let content:ContentItem[] = [];
  let results:ContentResult[] = [];

  // ── Assistente ──
  let setup = {
    server_name:'Meu Servidor Minecraft',
    server_dir:'~/Minecraft/Servidor',
    memory_limit_gb:4,
    loader:'vanilla',
  };
  let setupVersion = '';

  // ── Estado de UI ──
  let loading = false;
  let notice = '';
  let saved = false;
  let command = '';
  let booted = false;

  // ── Exclusão do servidor (2 fatores) ──
  let deleteOpen = false;
  let deleteStep: 1 | 2 = 1;
  let deleteName = '';
  let deleteAck = false;
  let deleting = false;

  // ── Catálogo estilo Prism ──
  let catalogOpen = false;
  let catalogDetail:ContentResult | null = null;
  let catalogLoading = false;
  let catalogError = '';
  let catalogQuery = '';

  // ── Helpers de API ──
  const api = async <T=unknown>(path:string, options?:RequestInit):Promise<T> => {
    const response = await fetch(`/api${path}`, options);
    const text = await response.text();
    let data:any = {};
    try { data = text ? JSON.parse(text) : {}; }
    catch { throw new Error(`Resposta inválida (${response.status})`); }
    if (!response.ok) throw new Error(
      typeof data.error === 'string' ? data.error : `Erro ${response.status}`
    );
    return data as T;
  };
  const json = (value:unknown):RequestInit => ({
    method:'POST',
    headers:{'Content-Type':'application/json'},
    body:JSON.stringify(value)
  });

  // ── Refresh / loaders ──
  async function refresh() {
    try {
      const fresh = await api<Server>('/state');
      server = fresh;
      if (booted === false) {
        booted = true;
        if (!fresh.installed) activeTab = 'setup';
      }
      if (!contentVersion && fresh.version) contentVersion = fresh.version;
      if (fresh.e4all?.loader) contentLoader = fresh.e4all.loader;
    } catch {
      notice = 'Núcleo Rust offline ou ainda iniciando.';
    }
  }

  async function loadVersions() {
    try {
      const data = await api<{versions:Version[]}>('/minecraft/versions');
      versions = data.versions.filter(v => v.kind === 'release');
      if (!setupVersion && versions.length) {
        setupVersion = server.version || versions[0].id;
      }
      if (!contentVersion && versions.length) {
        contentVersion = server.version || versions[0].id;
      }
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Não foi possível ler versões.';
    }
  }

  async function loadContent() {
    try {
      const data = await api<{items:ContentItem[]}>(`/content?kind=${contentKind}`);
      content = data.items;
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao listar conteúdo.';
    }
  }

  // ── Ações do servidor ──
  async function createServer() {
    loading = true;
    notice = '';
    try {
      server = await api<Server>('/server/create', json({
        server_name: setup.server_name,
        version: setupVersion,
        loader: setup.loader,
        server_dir: setup.server_dir,
        memory_limit_gb: setup.memory_limit_gb,
      }));
      activeTab = 'overview';
      notice = 'Servidor instalado. Aceite a EULA antes de iniciar.';
      await loadContent();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha na instalação.';
    }
    loading = false;
  }

  async function acceptEula() {
    try {
      await api('/server/eula', {method:'POST'});
      notice = 'EULA aceita. Você já pode iniciar o servidor.';
      await refresh();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao aceitar EULA.';
    }
  }

  async function saveConfig() {
    try {
      server = await api<Server>('/config', json({
        server_name: server.server_name,
        version: server.version,
        server_dir: server.server_dir,
        memory_limit_gb: server.memory_limit_gb,
      }));
      saved = true;
      setTimeout(()=>saved=false, 2200);
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao salvar.';
    }
  }

  async function toggleServer() {
    loading = true;
    try {
      server = await api<Server>(
        server.status==='online' ? '/server/stop' : '/server/start',
        {method:'POST'}
      );
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao controlar servidor.';
    }
    loading = false;
  }

  async function sendCommand() {
    if(!command.trim()) return;
    try {
      await api('/command', json({command}));
      command='';
      await refresh();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Console indisponível.';
    }
  }

  // ── Conteúdo ──
  async function installContent(project_id:string) {
    try {
      const version = contentVersion || server.version;
      await api('/content/install', json({
        project_id, kind:contentKind, loader:contentLoader, version,
      }));
      notice = 'Conteúdo instalado.';
      await loadContent();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao instalar conteúdo.';
    }
  }

  async function deleteContent(filename:string) {
    if(!confirm(`Excluir ${filename}?`)) return;
    try {
      await api('/content/delete', json({kind:contentKind, filename}));
      await loadContent();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao excluir.';
    }
  }

  // ── Catálogo ──
  async function openCatalog() {
    catalogOpen = true;
    await runCatalogSearch();
  }
  async function runCatalogSearch() {
    catalogLoading = true;
    catalogError = '';
    try {
      const version = contentVersion || server.version;
      const data = await api<{hits:ContentResult[]}>('/content/search', json({
        query: catalogQuery, kind: contentKind, loader: contentLoader, version,
      }));
      results = data.hits || [];
    } catch (e) {
      catalogError = e instanceof Error ? e.message : 'Falha na busca.';
    }
    catalogLoading = false;
  }
  function pickCategory(kind:'mods'|'plugins'|'datapacks') {
    contentKind = kind;
    loadContent();
    if (catalogOpen) runCatalogSearch();
  }

  // ── Exclusão ──
  function openDelete() {
    deleteOpen = true;
    deleteStep = 1;
    deleteName = '';
    deleteAck = false;
  }
  function closeDelete() {
    if (deleting) return;
    deleteOpen = false;
  }
  async function confirmDelete() {
    deleting = true;
    try {
      server = await api<Server>('/server/delete', json({
        confirm_name: deleteName.trim(),
      }));
      deleteOpen = false;
      activeTab = 'setup';
      notice = 'Servidor excluído. Crie uma nova instalação.';
    } catch (e) {
      notice = e instanceof Error ? e.message : 'Falha ao excluir servidor.';
    }
    deleting = false;
  }

  // ── Utilidades ──
  function formatBytes(n:number) {
    return n < 1024*1024
      ? `${Math.max(1,Math.round(n/1024))} KB`
      : `${(n/1024/1024).toFixed(1)} MB`;
  }
  function uptime(n:number) {
    const h=Math.floor(n/3600).toString().padStart(2,'0'),
          m=Math.floor(n%3600/60).toString().padStart(2,'0'),
          s=Math.floor(n%60).toString().padStart(2,'0');
    return `${h}:${m}:${s}`;
  }

  // ── Reatividade ──
  $: if (activeTab === 'content') loadContent();
  $: if (contentKind || contentVersion || contentLoader) results = [];

  onMount(() => {
    refresh();
    loadVersions();
    const timer = setInterval(refresh, 3000);
    return ()=>clearInterval(timer);
  });
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="mark">S</div>
      <div><strong>Shadow<span>Painel</span></strong><small>SERVER CONTROL</small></div>
    </div>
    <div class="workspace">
      <div class="server-avatar">☾</div>
      <div>
        <b>{server.server_name}</b>
        <small>{server.installed ? (server.version || 'INSTALADO') : 'SEM SERVIDOR'}</small>
      </div>
      <button class="dots">•••</button>
    </div>
    <nav>
      <div class="nav-label">SERVIDOR</div>
      <button class:active={activeTab==='overview'} on:click={()=>activeTab='overview'}>
        <span class="nav-icon">◈</span>Visão geral
      </button>
      <button class:active={activeTab==='content'} on:click={()=>activeTab='content'}>
        <span class="nav-icon">▦</span>Mods, plugins e datapacks
      </button>
      <button class:active={activeTab==='setup'} on:click={()=>activeTab='setup'}>
        <span class="nav-icon">＋</span>Criar servidor
      </button>
      <div class="nav-label settings-label">SISTEMA</div>
      <button class:active={activeTab==='config'} on:click={()=>activeTab='config'}>
        <span class="nav-icon">⚙</span>Configurações avançadas
      </button>
    </nav>
    <div class="sidebar-footer">
      <div class="secure">
        <span>●</span>
        <div><b>Execução local</b><small>Seus dados ficam no PC</small></div>
      </div>
      <div class="user">
        <div class="avatar">NP</div>
        <div><b>Administrador</b><small>ShadowPainel v{APP_VERSION}</small></div>
      </div>
    </div>
  </aside>

  <main class="main">
    <header>
      <div>
        <div class="breadcrumb">SERVIDORES <span>/</span> {server.server_name}</div>
        <h1>
          {activeTab==='content' ? 'Conteúdo do servidor'
           : activeTab==='config' ? 'Configurações avançadas'
           : activeTab==='setup' ? 'Criar servidor'
           : 'Olá, administrador'} <span class="wave">✦</span>
        </h1>
        <p>
          {activeTab==='content'
            ? 'Gerencie seus arquivos como no Prism Launcher.'
            : activeTab==='config'
              ? 'A instalação inicial fica separada das preferências do servidor.'
              : activeTab==='setup'
                ? 'Escolha versão, modloader e pasta para começar.'
                : 'Gerencie seu mundo sem abrir o terminal.'}
        </p>
      </div>
      <div class="header-actions">
        <button class="icon-button">◔</button>
        <button class="help">? <span>Ajuda</span></button>
      </div>
    </header>

    {#if notice}<div class="notice">{notice}</div>{/if}
    {#if saved}<div class="notice success-notice">Configuração salva.</div>{/if}

    {#if activeTab==='overview'}
      {#if !server.installed}
        <section class="settings-card setup-card">
          <div class="card-head">
            <div>
              <h3>Nenhum servidor instalado ainda</h3>
              <p>Crie um servidor para começar. Escolha a versão, o modloader e a pasta onde os arquivos serão baixados.</p>
            </div>
            <span class="mod-badge">PRIMEIROS PASSOS</span>
          </div>
          <button class="primary" on:click={()=>activeTab='setup'}>
            ABRIR ASSISTENTE DE CRIAÇÃO <span>→</span>
          </button>
        </section>
      {:else}
        <div class="hero">
          <div class="hero-copy">
            <div class="eyebrow">
              <span class:online={server.status==='online'} class="status-dot"></span>
              {server.status==='online'?'SERVIDOR ONLINE':'SERVIDOR OFFLINE'}
            </div>
            <h2>{server.server_name}</h2>
            <p>Minecraft Java Edition · {server.version || 'sem versão instalada'}</p>
          </div>
          <div class="orb"><div class="orb-inner">☾</div><i></i><i></i></div>
          
          <div class="hero-bottom">
            <div class="hero-actions">
              <button class="primary" on:click={toggleServer} disabled={loading}>
                {loading?'CARREGANDO...':server.status==='online'?'PARAR SERVIDOR':'INICIAR SERVIDOR'} <span>→</span>
              </button>
              <button class="secondary" on:click={acceptEula}>ACEITAR EULA</button>
              <button class="secondary" on:click={()=>activeTab='setup'}>＋ Nova instalação</button>
              <button class="secondary danger-outline" on:click={openDelete}>EXCLUIR SERVIDOR</button>
            </div>
            <div class="hero-meta">
              <div><span>ENDEREÇO</span><b>{server.address}</b></div>
              <div class="divider"></div>
              <div><span>UPTIME</span><b>{uptime(server.uptime_seconds)}</b></div>
            </div>
          </div>
        </div>

        <section class="metrics">
          <div class="metric">
            <div class="metric-icon blue">♙</div>
            <div><span>JOGADORES ONLINE</span><strong>{server.players}</strong></div>
          </div>
          <div class="metric">
            <div class="metric-icon purple">▣</div>
            <div><span>MEMÓRIA UTILIZADA</span>
              <strong>{server.memory_mb} <em>/ {server.memory_limit_gb*1024} MB</em></strong>
            </div>
          </div>
          <div class="metric">
            <div class="metric-icon green">◔</div>
            <div><span>USO DE CPU</span><strong>{server.cpu_percent.toFixed(1)}<em>%</em></strong></div>
          </div>
        </section>

        <div class="content-grid">
          <section class="console-card">
            <div class="card-head">
              <div><h3>Console do servidor</h3><p>Logs em tempo real</p></div>
              <div class="live"><span></span> AO VIVO</div>
            </div>
            <div class="console">
              {#each server.logs as log}
                <div class="log">
                  <span class="time">{log.time}</span>
                  <span class={log.kind}>{log.text}</span>
                </div>
              {/each}
            </div>
            <div class="command-line">
              <span>›</span>
              <input bind:value={command} on:keydown={(e)=>e.key==='Enter'&&sendCommand()} placeholder="Digite um comando..."/>
              <button on:click={sendCommand}>Enviar ↗</button>
            </div>
          </section>
          <section class="quick-card">
            <div class="card-head">
              <div><h3>Ações rápidas</h3><p>Atalhos de administração</p></div>
            </div>
            <button class="quick" on:click={toggleServer} disabled={loading}>
              <span class="q-icon">{server.status==='online'?'■':'▶'}</span>
              <div><b>{server.status==='online'?'Parar':'Iniciar'} servidor</b>
                <small>Controle seguro do processo Java</small></div>
              <span>›</span>
            </button>
            <button class="quick" on:click={()=>activeTab='content'}>
              <span class="q-icon">▦</span>
              <div><b>Gerenciar conteúdo</b><small>Mods, plugins e datapacks</small></div>
              <span>›</span>
            </button>
            <button class="quick" on:click={()=>activeTab='config'}>
              <span class="q-icon">⚙</span>
              <div><b>Configurações avançadas</b><small>Memória, pasta e e4all</small></div>
              <span>›</span>
            </button>
          </section>
        </div>
      {/if}

    {:else if activeTab==='content'}
      <section class="settings-card content-manager">
        <div class="card-head">
          <div>
            <h3>Conteúdo do servidor</h3>
            <p>Gerencie {contentKind} como no Prism Launcher: veja o que está instalado e explore o catálogo por categoria.</p>
          </div>
          <button class="primary" on:click={openCatalog}>
            ＋ Adicionar {contentKind === 'mods' ? 'mods' : contentKind === 'plugins' ? 'plugins' : 'datapacks'}
          </button>
        </div>

        <div class="content-tabs">
          {#each [['mods','Mods'],['plugins','Plugins'],['datapacks','Datapacks']] as item}
            <button class:active={contentKind===item[0]}
                    on:click={()=>pickCategory(item[0] as 'mods'|'plugins'|'datapacks')}>
              {item[1]}
              {#if contentKind===item[0] && content.length}
                <span class="tab-count">{content.length}</span>
              {/if}
            </button>
          {/each}
        </div>

        <div class="content-filters">
          <label>
            Versão Minecraft
            <select bind:value={contentVersion}>
              {#each versions as version}<option value={version.id}>{version.id}</option>{/each}
            </select>
            <small class="selected-version">Compatível com: {contentVersion || '—'}</small>
          </label>
          <label>
            Modloader
            <select bind:value={contentLoader}>
              <option value="fabric">Fabric</option>
              <option value="forge">Forge</option>
              <option value="neoforge">NeoForge</option>
              <option value="quilt">Quilt</option>
              <option value="paper">Paper</option>
              <option value="purpur">Purpur</option>
              <option value="vanilla">Vanilla</option>
            </select>
          </label>
          <label>
            Categoria
            <select bind:value={contentKind} on:change={()=>loadContent()}>
              <option value="mods">Mods</option>
              <option value="plugins">Plugins</option>
              <option value="datapacks">Datapacks</option>
            </select>
          </label>
        </div>

        <div class="content-section-head">
          <h4>Instalados <span>{content.length}</span></h4>
        </div>

        {#if content.length===0}
          <div class="empty">
            Nenhum arquivo em <b>{contentKind}/</b> ainda.
            <br/>Clique em <b>Adicionar {contentKind}</b> para explorar o catálogo.
          </div>
        {:else}
          <div class="installed-grid">
            {#each content as file}
              <div class="installed-card">
                <span class="file-icon">{contentKind==='mods'?'◆':contentKind==='plugins'?'⬡':'◇'}</span>
                <div class="installed-meta">
                  <b>{file.filename}</b>
                  <small>{formatBytes(file.size)} · {contentKind}</small>
                </div>
                <button class="danger small-button" on:click={()=>deleteContent(file.filename)}>Remover</button>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      {#if catalogOpen}
        <div class="modal-overlay" on:click|self={()=>{catalogOpen=false;catalogDetail=null;}}>
          <div class="modal catalog-modal">
            <div class="modal-head">
              <div>
                <h3>Adicionar {contentKind}</h3>
                <p>Explore por nome, autor ou descrição. Filtros: {contentVersion || '—'} · {contentLoader}</p>
              </div>
              <button class="modal-close" on:click={()=>{catalogOpen=false;catalogDetail=null;}}>✕</button>
            </div>

            <div class="catalog-search">
              <input bind:value={catalogQuery}
                     placeholder="Pesquisar no Modrinth (ex.: sodium, worldedit)..."
                     on:keydown={(e)=>e.key==='Enter'&&runCatalogSearch()}/>
              <button class="primary" on:click={runCatalogSearch}
                      disabled={catalogLoading || !contentVersion}>
                {catalogLoading?'Buscando...':'Buscar'}
              </button>
            </div>

            {#if catalogError}<div class="notice">{catalogError}</div>{/if}

            {#if catalogLoading}
              <div class="catalog-loading"><div class="spinner"></div> Consultando Modrinth…</div>
            {:else if results.length===0}
              <div class="empty">
                Nenhum resultado para <b>{contentVersion}</b> em <b>{contentLoader}</b>.
                <br/>Tente outra versão, outro loader ou um termo diferente.
              </div>
            {:else}
              <div class="catalog-grid">
                {#each results as item}
                  <div class="catalog-card" role="button" tabindex="0"
                       on:click={()=>catalogDetail=item}
                       on:keydown={(e)=>(e.key==='Enter'||e.key===' ')&&(catalogDetail=item)}>
                    <div class="catalog-icon">
                      {#if item.icon_url}
                        <img src={item.icon_url} alt="" loading="lazy"/>
                      {:else}
                        <span>{item.title.slice(0,1).toUpperCase()}</span>
                      {/if}
                    </div>
                    <div class="catalog-text">
                      <b>{item.title}</b>
                      <small>{item.description || 'Projeto Modrinth'}</small>
                      <span class="catalog-downloads">⇩ {item.downloads.toLocaleString('pt-BR')}</span>
                    </div>
                    <button class="primary small-button"
                            on:click|stopPropagation={()=>installContent(item.project_id)}>
                      Instalar
                    </button>
                  </div>
                {/each}
              </div>
            {/if}

            {#if catalogDetail}
              <div class="detail-sheet">
                <div class="detail-head">
                  <div class="catalog-icon big">
                    {#if catalogDetail.icon_url}
                      <img src={catalogDetail.icon_url} alt=""/>
                    {:else}
                      <span>{catalogDetail.title.slice(0,1).toUpperCase()}</span>
                    {/if}
                  </div>
                  <div>
                    <h4>{catalogDetail.title}</h4>
                    <small>{catalogDetail.downloads.toLocaleString('pt-BR')} downloads</small>
                  </div>
                  <button class="modal-close" on:click={()=>catalogDetail=null}>✕</button>
                </div>
                <p>{catalogDetail.description || 'Sem descrição fornecida.'}</p>
                <div class="detail-actions">
                  <button class="primary"
                          on:click={()=>{installContent(catalogDetail!.project_id);catalogDetail=null;}}>
                    Instalar em {contentVersion}
                  </button>
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}

    {:else if activeTab==='config'}
      <section class="settings-card">
        <div class="card-head">
          <div>
            <h3>Configurações avançadas</h3>
            <p>Estas opções ficam separadas do assistente de instalação inicial.</p>
          </div>
        </div>
        <div class="form-grid">
          <label>Nome do servidor<input bind:value={server.server_name}/></label>
          <label>Versão atual<input bind:value={server.version}/></label>
          <label>Pasta do servidor<input bind:value={server.server_dir}/></label>
          <label>Memória (GB)<input type="number" min="1" max="64" bind:value={server.memory_limit_gb}/></label>
        </div>
        <div class="java-check">
          <span class:ok={server.java_detected}>●</span>
          <div>
            <b>{server.java_detected?'Java detectado':'Java não detectado'}</b>
            <small>{server.java_path} · necessário para iniciar</small>
          </div>
        </div>
        <button class="primary" on:click={saveConfig}>SALVAR CONFIGURAÇÃO <span>→</span></button>
      </section>

      <section class="settings-card">
        <div class="card-head">
          <div><h3>e4all</h3><p>Configurações complementares do servidor.</p></div>
        </div>
        <div class="form-grid">
          <label>Loader
            <select bind:value={server.e4all.loader}>
              <option value="fabric">Fabric</option>
              <option value="forge">Forge</option>
              <option value="neoforge">NeoForge</option>
              <option value="quilt">Quilt</option>
            </select>
          </label>
          <label>Versão Minecraft<input bind:value={server.e4all.minecraft_version}/></label>
          <label>Pasta de instalação<input bind:value={server.e4all.install_path}/></label>
          <label>Endereço público<input bind:value={server.e4all.public_address}/></label>
        </div>
        <button class="primary"
          on:click={async()=>{
            try {
              server = await api<Server>('/e4all/config', json(server.e4all));
              saved = true; setTimeout(()=>saved=false, 2200);
            } catch(e) {
              notice = e instanceof Error ? e.message : 'Falha ao salvar.';
            }
          }}>
          SALVAR e4all <span>→</span>
        </button>
      </section>

    {:else if activeTab==='setup'}
      <section class="settings-card setup-card">
        <div class="card-head">
          <div>
            <h3>Instalação inicial do servidor</h3>
            <p>Escolha a versão do Minecraft, o tipo de servidor e a pasta. O ShadowPainel baixa e prepara os arquivos automaticamente.</p>
          </div>
          <span class="mod-badge">ASSISTENTE</span>
        </div>

        <div class="setup-steps">
          <span class="current">1. Identidade</span>
          <span>2. Versão e loader</span>
          <span>3. Conteúdo filtrado por versão</span>
        </div>

        <div class="form-grid">
          <label>
            Nome do servidor
            <input bind:value={setup.server_name} placeholder="Sobrevivência principal"/>
            <small style="color: var(--faint); font-size: 11px; margin-top: 4px;">
              Este nome aparece no painel. Pode ser alterado depois.
            </small>
          </label>
          <label>
            Pasta do servidor
            <input bind:value={setup.server_dir}/>
            <small style="color: var(--faint); font-size: 11px; margin-top: 4px;">
              Onde os arquivos do mundo serão salvos. Use o caminho completo.
            </small>
          </label>
          <label>
            Versão do Minecraft
            <select bind:value={setupVersion}>
              {#each versions as version}
                <option value={version.id}>{version.id}</option>
              {/each}
            </select>
            <small class="selected-version">
              Selecione a versão que você joga. Mods e plugins dependem dela.
            </small>
          </label>
          <label>
            Tipo / modloader
            <select bind:value={setup.loader}>
              <option value="vanilla">Vanilla — servidor oficial (recomendado para iniciantes)</option>
              <option value="fabric">Fabric — para usar mods</option>
              <option value="paper">Paper — para usar plugins (melhor desempenho)</option>
              <option value="purpur">Purpur — plugins + customizações</option>
            </select>
            <small class="selected-version">
              Se não for usar mods ou plugins, escolha Vanilla.
            </small>
          </label>
          <label>Memória inicial (GB)
            <input type="number" min="1" max="64" bind:value={setup.memory_limit_gb}/>
          </label>
        </div>

        <div class="setup-note">
          <b>Depois da instalação</b>
          <span>Você poderá administrar mods, plugins e datapacks na área de conteúdo, com busca por versão no Modrinth.</span>
        </div>

        <button class="primary"
          on:click={createServer}
          disabled={loading || !setupVersion}>
          {#if loading}
            <span class="spinner" style="width: 14px; height: 14px; border-width: 2px; margin-right: 8px;"></span>
            BAIXANDO ARQUIVOS...
          {:else}
            INSTALAR E ABRIR PAINEL <span>→</span>
          {/if}
        </button>
      </section>
    {/if}

    {#if deleteOpen}
      <div class="modal-overlay" on:click|self={closeDelete}>
        <div class="modal danger-modal">
          <div class="modal-head">
            <div>
              <h3>Excluir servidor</h3>
              <p>Esta ação é permanente e apaga <b>todos</b> os arquivos da pasta do servidor.</p>
            </div>
            <button class="modal-close" on:click={closeDelete}>✕</button>
          </div>

          {#if deleteStep === 1}
            <div class="danger-summary">
              <div><span>Servidor</span><b>{server.server_name}</b></div>
              <div><span>Versão</span><b>{server.version || '—'}</b></div>
              <div><span>Pasta</span><b class="mono">{server.server_dir}</b></div>
            </div>
            <label class="check-label">
              <span>
                <input type="checkbox" bind:checked={deleteAck}/>
                Entendo que os arquivos serão apagados permanentemente.
              </span>
              <small>Não é possível desfazer.</small>
            </label>
            <div class="modal-actions">
              <button class="secondary" on:click={closeDelete}>Cancelar</button>
              <button class="danger" disabled={!deleteAck} on:click={()=>deleteStep=2}>
                Continuar
              </button>
            </div>
          {:else}
            <p class="danger-warning">
              Para confirmar, digite exatamente o nome do servidor:
              <br/><b class="mono">{server.server_name}</b>
            </p>
            <input class="danger-input" bind:value={deleteName}
                   placeholder={server.server_name} autocomplete="off"/>
            <div class="modal-actions">
              <button class="secondary" on:click={()=>deleteStep=1} disabled={deleting}>Voltar</button>
              <button class="danger"
                disabled={deleting || deleteName.trim() !== server.server_name.trim()}
                on:click={confirmDelete}>
                {deleting ? 'EXCLUINDO...' : 'EXCLUIR DEFINITIVAMENTE'}
              </button>
            </div>
          {/if}
        </div>
      </div>
    {/if}

    <footer>
      <span>SHADOWPAINEL <b>v{APP_VERSION}</b></span>
      <span>RUST CORE · MULTIPLATAFORMA <i>●</i></span>
    </footer>
  </main>
</div>