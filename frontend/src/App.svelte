<script lang="ts">
  import { onMount } from 'svelte';
  const APP_VERSION = '0.6.5';
  type Log = { time:string; text:string; kind:string };
  type E4allConfig = { loader:string; minecraft_version:string; android_variant:boolean; offline_accounts:boolean; voice_chat:boolean; install_path:string; public_address:string };
  type ServerState = { status:string; address:string; players:number; memory_mb:number; memory_limit_gb:number; cpu_percent:number; uptime_seconds:number; server_name:string; version:string; server_dir:string; java_path:string; java_detected:boolean; e4all:E4allConfig; logs:Log[] };
  
  let server: ServerState = { 
    status:'offline', address:'localhost:25565', players:0, memory_mb:0, memory_limit_gb:4, cpu_percent:0, uptime_seconds:0, server_name:'Sobrevivência principal', version:'1.21.1', server_dir:'~/Minecraft/MeuServidor', java_path:'java', java_detected:true, e4all:{loader:'fabric',minecraft_version:'1.21.1',android_variant:false,offline_accounts:false,voice_chat:true,install_path:'~/Minecraft/MeuServidor/mods',public_address:''}, logs:[] 
  };
  
  let command = ''; 
  let activeTab = 'Visão geral'; 
  let loading = false; 
  let notice = ''; 
  let saved = false;

  // Wizard de Criação
  let wizardStep = 0;
  let wizardType = 'vanilla';
  let wizardVersion = '1.21.1';
  let wizardMemory = 4;

  // Modal de Mods
  let showModBrowser = false;
  let browserType = 'mod';
  let searchQuery = '';
  let searchResults: any[] = [];
  let isSearching = false;
  let selectedMod: any = null;
  let installingMod = false;

  // Gerenciador de Arquivos
  let installedFiles: any[] = [];
  let managingType = 'mod';

  const api = async <T = unknown>(path:string, options?:RequestInit):Promise<T> => { 
    const r = await fetch(`/api${path}`, options); 
    const text = await r.text(); 
    let data:Record<string, unknown> = {}; 
    try { data = text ? JSON.parse(text) : {}; } catch { throw new Error(`Resposta inválida do núcleo (${r.status})`); } 
    if (!r.ok) throw new Error(typeof data.error === 'string' ? data.error : `Erro ${r.status}`); 
    return data as T; 
  };

  async function refresh() { 
    try { server = await api<ServerState>('/state'); } catch { notice = 'Modo demonstração: inicie o núcleo Rust.'; } 
  }

  async function toggleServer() { 
    loading = true; notice=''; 
    try { server = await api<ServerState>(server.status === 'online' ? '/server/stop' : '/server/start', {method:'POST'}); } 
    catch(e) { notice = e instanceof Error ? e.message : 'Não foi possível conectar.'; } 
    loading=false; 
  }

  async function restartServer() { 
    loading = true; notice=''; 
    try { 
      if (server.status === 'online') { 
        await api('/server/stop', {method:'POST'}); 
        for (let attempt=0; attempt<40; attempt++) { await new Promise(r => setTimeout(r, 500)); server = await api<ServerState>('/state'); if (server.status === 'offline') break; } 
      } 
      server = await api<ServerState>('/server/start', {method:'POST'}); 
    } catch(e) { notice = e instanceof Error ? e.message : 'Falha ao reiniciar.'; } 
    loading=false; 
  }

  async function saveConfig() { 
    try { 
      server = await api<ServerState>('/config', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({server_name:server.server_name, version:server.version, server_dir:server.server_dir, memory_limit_gb:server.memory_limit_gb})}); 
      saved=true; setTimeout(()=>saved=false, 2200); 
    } catch(e) { notice=e instanceof Error ? e.message : 'Falha ao salvar'; } 
  }

  async function saveE4all() { 
    try { 
      server = await api<ServerState>('/e4all/config', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(server.e4all)}); 
      saved=true; setTimeout(()=>saved=false, 2200); 
    } catch(e) { notice=e instanceof Error ? e.message : 'Falha ao salvar e4all'; } 
  }

  async function sendCommand() { 
    if (!command.trim()) return; 
    try { 
      await api('/command', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({command})}); 
      command=''; await refresh(); 
    } catch { notice='Console disponível quando online.'; } 
  }

  async function createServer() {
    wizardStep = 3;
    notice = 'Baixando e preparando o servidor...';
    try {
      if (wizardType === 'vanilla') {
        await api('/server/download', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({version: wizardVersion}) });
        await api('/server/eula', { method: 'POST' });
      } else if (wizardType === 'fabric') {
        await api('/server/download', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({version: wizardVersion}) });
        await api('/server/install-fabric', { method: 'POST' });
        await api('/server/eula', { method: 'POST' });
      }
      await api('/config', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({server_name:server.server_name, version:wizardVersion, server_dir:server.server_dir, memory_limit_gb:wizardMemory})});
      await refresh();
      notice = 'Servidor criado com sucesso!';
      wizardStep = 0;
      activeTab = 'Visão geral';
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao criar servidor.';
      wizardStep = 0;
    }
  }

  async function openBrowser(type: string) {
    browserType = type;
    showModBrowser = true;
    searchQuery = '';
    searchResults = [];
    selectedMod = null;
    await doSearch();
  }

  async function doSearch() {
    isSearching = true;
    try {
      let loader = 'fabric';
      if (browserType === 'plugin') loader = 'paper';
      if (browserType === 'datapack') loader = 'datapack';

      const data = await api<any>('/mods/search', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({
          query: searchQuery,
          loader: loader,
          version: server.version,
          project_type: browserType
        })
      });
      searchResults = data.hits || [];
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha na busca.';
    }
    isSearching = false;
  }

  async function confirmInstall() {
    if (!selectedMod) return;
    installingMod = true;
    notice = 'Instalando...';
    
    let loader = 'fabric';
    if (browserType === 'plugin') loader = 'paper';
    if (browserType === 'datapack') loader = 'datapack';

    try {
      await api('/mods/install', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({
          project_id: selectedMod.project_id,
          loader: loader,
          version: server.version,
          project_type: browserType
        })
      });
      notice = `${selectedMod.title} instalado com sucesso!`;
      showModBrowser = false;
      selectedMod = null;
      await loadInstalled(managingType);
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao instalar.';
    }
    installingMod = false;
  }

  async function loadInstalled(type: string) {
    managingType = type;
    try {
      const data = await api<any>('/files/list', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ project_type: type })
      });
      installedFiles = data.files || [];
    } catch(e) { installedFiles = []; }
  }

  async function deleteFile(filename: string) {
    if (!confirm(`Excluir ${filename}?`)) return;
    try {
      await api('/files/delete', {
        method: 'POST',
        headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({ project_type: managingType, filename })
      });
      await loadInstalled(managingType);
      notice = 'Arquivo excluído.';
    } catch(e) { notice = e instanceof Error ? e.message : 'Falha ao excluir.'; }
  }

  function uptime(sec:number) { 
    const h=Math.floor(sec/3600).toString().padStart(2,'0'), m=Math.floor(sec%3600/60).toString().padStart(2,'0'), s=Math.floor(sec%60).toString().padStart(2,'0'); 
    return `${h}:${m}:${s}`; 
  }

  onMount(() => { refresh(); loadInstalled('mod'); const timer=setInterval(refresh, 3000); return () => clearInterval(timer); });
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="mark">S</div>
      <div><strong>Shadow<span>Painel</span></strong><small>SERVER CONTROL</small></div>
    </div>
    <div class="workspace">
      <div class="server-avatar">☾</div>
      <div><b>Meu servidor</b><small>LOCAL · PRINCIPAL</small></div>
      <button class="dots" aria-label="Mais opções" title="Mais opções">•••</button>
    </div>
    <nav>
      <div class="nav-label">GERENCIAMENTO</div>
      {#each ['Visão geral'] as tab}
        <button class:active={activeTab===tab} on:click={() => activeTab=tab}>
          <span class="nav-icon">{tab==='Visão geral'?'◈':tab==='Console'?'>_':tab==='Jogadores'?'♙':'▤'}</span>
          {tab}
        </button>
      {/each}
      <div class="nav-label settings-label">SISTEMA</div>
      <button class:active={activeTab==='e4all'} on:click={() => activeTab='e4all'}><span class="nav-icon">⬡</span>e4all</button>
      <button class:active={activeTab==='Configurações'} on:click={() => activeTab='Configurações'}><span class="nav-icon">⚙</span>Configurações</button>
    </nav>
    <div class="sidebar-footer">
      <div class="secure"><span>●</span><div><b>Execução local</b><small>Seus dados ficam no PC</small></div></div>
      <div class="user"><div class="avatar">NP</div><div><b>Nova instalação</b><small>Administrador</small></div><span>⌄</span></div>
    </div>
  </aside>

  <main class="main">
    <header>
      <div>
        <div class="breadcrumb">SERVIDORES <span>/</span> MEU SERVIDOR</div>
        <h1>Olá, administrador <span class="wave">✦</span></h1>
        <p>Gerencie seu mundo sem abrir o terminal.</p>
      </div>
      <div class="header-actions"><button class="icon-button" aria-label="Tema" title="Tema">◔</button><button class="help" aria-label="Ajuda" title="Ajuda">? <span>Ajuda</span></button></div>
    </header>

    {#if notice}<div class="notice">{notice}</div>{/if}
    {#if saved}<div class="notice success-notice">Configuração salva no núcleo Rust.</div>{/if}

    {#if activeTab === 'e4all'}
      <section class="settings-card">
        <div class="card-head">
          <div><h3>Configuração do e4all</h3><p>Instalação e preferências do mod de conexão LAN pela internet</p></div>
          <span class="mod-badge">MODRINTH · e4all</span>
        </div>
        <div class="form-grid">
          <label>Mod loader<select bind:value={server.e4all.loader}><option value="fabric">Fabric</option><option value="forge">Forge</option><option value="neoforge">NeoForge</option><option value="quilt">Quilt</option></select></label>
          <label>Versão do Minecraft<input bind:value={server.e4all.minecraft_version} placeholder="1.21.1"></label>
          <label>Pasta mods<input bind:value={server.e4all.install_path}></label>
          <label>Domínio público e4all<input bind:value={server.e4all.public_address} placeholder="exemplo.e4mc.link"><button class="copy-field" aria-label="Copiar domínio público" on:click={() => navigator.clipboard?.writeText(server.e4all.public_address)}>Copiar</button></label>
          <label class="check-label"><span><input type="checkbox" bind:checked={server.e4all.android_variant}> Variante Android</span><small>Use o JAR maior com sufixo -android</small></label>
        </div>
        <div class="toggle-grid">
          <label><input type="checkbox" bind:checked={server.e4all.offline_accounts}><span><b>Contas offline/cracked</b><small>Ativa o modo Online Mode desligado no mundo LAN</small></span></label>
          <label><input type="checkbox" bind:checked={server.e4all.voice_chat}><span><b>Simple Voice Chat</b><small>Voz P2P quando todos os jogadores têm e4all + SVC</small></span></label>
        </div>
        <div class="e4all-note">
          <b>Como conectar pelo e4all</b>
          <p>O e4all funciona dentro do Minecraft: instale o mod apenas no computador host, abra um mundo e clique em <strong>Abrir para LAN</strong>. O próprio e4all gera o domínio no chat; cole esse domínio neste campo e compartilhe com os jogadores.</p>
        </div>
        <button class="primary" on:click={saveE4all}>SALVAR CONFIGURAÇÃO DO e4all <span>→</span></button>
      </section>

    {:else if activeTab === 'Configurações'}
      <section class="settings-card">
        <div class="card-head">
          <div><h3>Criar Novo Servidor</h3><p>Siga os passos para baixar e configurar seu servidor</p></div>
        </div>

        {#if wizardStep === 0}
          <div style="padding: 24px;">
            <p style="margin-bottom: 16px; color: var(--muted);">Escolha o tipo de servidor:</p>
            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 12px;">
              <button class="quick" on:click={() => { wizardType = 'vanilla'; wizardStep = 1; }} style="justify-content: center;">
                <span class="q-icon">🌍</span>
                <div style="text-align: center;"><b>Vanilla</b><small>Servidor puro da Mojang</small></div>
              </button>
              <button class="quick" on:click={() => { wizardType = 'fabric'; wizardStep = 1; }} style="justify-content: center;">
                <span class="q-icon">🧵</span>
                <div style="text-align: center;"><b>Fabric</b><small>Para mods e e4all</small></div>
              </button>
              <button class="quick" on:click={() => { wizardType = 'paper'; wizardStep = 1; }} style="justify-content: center;">
                <span class="q-icon">📄</span>
                <div style="text-align: center;"><b>Paper</b><small>Para plugins (em breve)</small></div>
              </button>
            </div>
          </div>

        {:else if wizardStep === 1}
          <div style="padding: 24px;">
            <p style="margin-bottom: 16px; color: var(--muted);">Versão do Minecraft:</p>
            <div class="form-grid" style="grid-template-columns: 1fr;">
              <label>Versão<input bind:value={wizardVersion} placeholder="1.21.1"></label>
            </div>
            <div style="display: flex; gap: 12px; margin-top: 16px;">
              <button class="secondary" on:click={() => wizardStep = 0}>Voltar</button>
              <button class="primary" on:click={() => wizardStep = 2}>Próximo →</button>
            </div>
          </div>

        {:else if wizardStep === 2}
          <div style="padding: 24px;">
            <p style="margin-bottom: 16px; color: var(--muted);">Configurações básicas:</p>
            <div class="form-grid">
              <label>Nome do servidor<input bind:value={server.server_name}></label>
              <label>Memória (GB)<input type="number" min="1" max="64" bind:value={wizardMemory}></label>
            </div>
            <div style="display: flex; gap: 12px; margin-top: 16px;">
              <button class="secondary" on:click={() => wizardStep = 1}>Voltar</button>
              <button class="primary" on:click={createServer}>Criar Servidor →</button>
            </div>
          </div>

        {:else if wizardStep === 3}
          <div style="padding: 48px; text-align: center;">
            <p style="color: var(--muted);">Baixando arquivos... Isso pode levar alguns minutos.</p>
            <div style="margin-top: 16px; width: 40px; height: 40px; border: 3px solid var(--border); border-top-color: var(--accent); border-radius: 50%; margin: 16px auto; animation: spin 1s linear infinite;"></div>
          </div>
          <style>@keyframes spin { to { transform: rotate(360deg); } }</style>
        {/if}
      </section>

      <section class="settings-card" style="margin-top: 24px;">
        <div class="card-head">
          <div><h3>Gerenciador de Conteúdo</h3><p>Veja, exclua ou adicione mods, plugins e datapacks</p></div>
        </div>
        
        <div style="display: flex; gap: 8px; margin-bottom: 16px;">
          <button class="secondary" on:click={() => loadInstalled('mod')}>Mods</button>
          <button class="secondary" on:click={() => loadInstalled('plugin')}>Plugins</button>
          <button class="secondary" on:click={() => loadInstalled('datapack')}>Datapacks</button>
          <button class="primary" style="margin-left: auto;" on:click={() => openBrowser(managingType)}>+ Adicionar</button>
        </div>

        {#if installedFiles.length === 0}
          <p style="color: var(--muted); text-align: center; padding: 24px;">Nenhum arquivo instalado. Clique em "+ Adicionar" para buscar.</p>
        {:else}
          <div style="display: flex; flex-direction: column; gap: 8px;">
            {#each installedFiles as file}
              <div style="display: flex; justify-content: space-between; align-items: center; background: var(--panel-2); border: 1px solid var(--border); border-radius: 8px; padding: 12px 16px;">
                <div>
                  <b style="color: #fff;">{file.name}</b>
                  <small style="color: var(--muted); margin-left: 12px;">{(file.size / 1024).toFixed(1)} KB · {file.modified}</small>
                </div>
                <button class="secondary" style="color: var(--red); border-color: rgba(255,92,92,0.3);" on:click={() => deleteFile(file.name)}>Excluir</button>
              </div>
            {/each}
          </div>
        {/if}
      </section>

    {:else if activeTab === 'Visão geral'}
      <div class="hero">
        <div class="hero-copy">
          <div class="eyebrow"><span class:online={server.status==='online'} class="status-dot"></span>{server.status==='online'?'SERVIDOR ONLINE':'SERVIDOR OFFLINE'}</div>
          <h2>{server.server_name}</h2>
          <p>Minecraft Java Edition · {server.version}</p>
        </div>
        <div class="orb"><div class="orb-inner">☾</div><i></i><i></i></div>
        <div class="hero-actions">
          <button class="primary" on:click={toggleServer} disabled={loading}>{loading?'CARREGANDO...':server.status==='online'?'PARAR SERVIDOR':'INICIAR SERVIDOR'} <span>→</span></button>
          <button class="secondary" on:click={() => activeTab='Configurações'}>⚙ Configurar</button>
        </div>
        <div class="hero-meta">
          <div><span>ENDEREÇO PARA COMPARTILHAR</span><b>{server.address}</b><button class="copy" aria-label="Copiar endereço" title="Copiar endereço" on:click={() => navigator.clipboard?.writeText(server.address)}>⧉</button></div>
          <div class="divider"></div>
          <div><span>UPTIME</span><b>{uptime(server.uptime_seconds)}</b></div>
        </div>
      </div>

      <section class="metrics">
        <div class="metric"><div class="metric-icon blue">♙</div><div><span>JOGADORES ONLINE</span><strong>{server.players} <em>/ 20</em></strong></div><div class="sparkline">╱╲╱╲╱</div></div>
        <div class="metric"><div class="metric-icon purple">▣</div><div><span>MEMÓRIA UTILIZADA</span><strong>{server.memory_mb} <em>/ {server.memory_limit_gb * 1024} MB</em></strong></div><div class="bar"><i style={`width:${Math.min(100, server.memory_mb/(server.memory_limit_gb*1024)*100)}%`}></i></div></div>
        <div class="metric"><div class="metric-icon green">◔</div><div><span>USO DE CPU</span><strong>{server.cpu_percent.toFixed(1)}<em>%</em></strong></div><div class="ring"><span></span></div></div>
      </section>

      <div class="content-grid">
        <section class="console-card">
          <div class="card-head"><div><h3>Console do servidor</h3><p>Logs em tempo real</p></div><div class="live"><span></span> AO VIVO</div></div>
          <div class="console">
            {#each server.logs as log}
              <div class="log"><span class="time">{log.time}</span><span class:success={log.kind==='success'} class:warning={log.kind==='warning'} class:command={log.kind==='command'}>{log.text}</span></div>
            {/each}
            {#if server.status==='online'}<div class="log"><span class="time">agora</span><span>Done! Type "help" for help</span></div>{/if}
          </div>
          <div class="command-line">
            <span>›</span>
            <input bind:value={command} on:keydown={(e) => e.key==='Enter' && sendCommand()} placeholder="Digite um comando..."/>
            <button on:click={sendCommand}>Enviar <span>↗</span></button>
          </div>
        </section>

        <section class="quick-card">
          <div class="card-head"><div><h3>Ações rápidas</h3><p>Atalhos de administração</p></div><button class="more">•••</button></div>
          <button class="quick" on:click={restartServer} disabled={loading}><span class="q-icon">↻</span><div><b>Reiniciar servidor</b><small>Parar com segurança e iniciar novamente</small></div><span>›</span></button>
          <button class="quick" disabled title="Backup automático será disponibilizado em breve"><span class="q-icon">↓</span><div><b>Fazer backup</b><small>Disponível em uma próxima versão</small></div><span>›</span></button>
          <button class="quick" on:click={() => activeTab='Configurações'}><span class="q-icon">⚙</span><div><b>Abrir configurações</b><small>Editar memória e versão</small></div><span>›</span></button>
          <div class="tip"><span>✦</span><div><b>Dica do ShadowPainel</b><p>Seus dados ficam locais e o painel não exige conta ou terminal.</p></div></div>
        </section>
      </div>
    {/if}

    <footer>
      <span>SHADOWPAINEL <b>v{APP_VERSION}</b></span>
      <span>RUST CORE · MULTIPLATAFORMA <i>●</i></span>
    </footer>
  </main>
</div>

{#if showModBrowser}
  <div class="modal-overlay">
    <div class="modal-content">
      <div class="modal-header">
        <h2>Baixar {browserType === 'mod' ? 'Mods' : browserType === 'plugin' ? 'Plugins' : 'Datapacks'}</h2>
        <button class="close-btn" on:click={() => showModBrowser = false}>✕</button>
      </div>
      
      <div class="modal-body">
        <div class="search-bar">
          <input bind:value={searchQuery} on:keydown={(e) => e.key === 'Enter' && doSearch()} placeholder="Procure por {browserType}s..." />
          <button class="primary" on:click={doSearch} disabled={isSearching}>
            {isSearching ? 'Buscando...' : 'Buscar'}
          </button>
        </div>

        <div class="results-layout">
          <div class="results-list">
            {#if isSearching}
              <p style="padding: 20px; text-align: center; color: var(--muted);">Buscando no Modrinth...</p>
            {:else if searchResults.length === 0}
              <p style="padding: 20px; text-align: center; color: var(--muted);">Nenhum resultado encontrado.</p>
            {:else}
              {#each searchResults as mod}
                <div class="result-item" class:selected={selectedMod?.project_id === mod.project_id} on:click={() => selectedMod = mod}>
                  {#if mod.icon_url}
                    <img src={mod.icon_url} alt="Icon" />
                  {:else}
                    <div class="icon-placeholder">?</div>
                  {/if}
                  <div class="result-info">
                    <b>{mod.title}</b>
                    <small>{mod.description}</small>
                  </div>
                </div>
              {/each}
            {/if}
          </div>

          <div class="details-panel">
            {#if selectedMod}
              <h3>{selectedMod.title}</h3>
              <p class="author">Por {selectedMod.author}</p>
              <div class="tags">
                <span class="tag">{selectedMod.downloads} downloads</span>
                <span class="tag">Versão: {server.version}</span>
              </div>
              
              <div class="actions">
                <button class="primary" on:click={confirmInstall} disabled={installingMod}>
                  {installingMod ? 'Instalando...' : 'Instalar'}
                </button>
              </div>
            {:else}
              <div class="empty-state">
                <p>Selecione um item para ver os detalhes</p>
              </div>
            {/if}
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}