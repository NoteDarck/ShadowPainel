<script lang="ts">
  import { afterUpdate, onMount } from 'svelte';

  const APP_VERSION = '0.7.8';

  // ── Tipos ──
  type Log = { time:string; text:string; kind:string };
  type Version = { id:string; kind:string; release_time:string };
  type ContentItem = {
    filename:string; size:number; kind:string; title?:string;
    project_id?:string; version_number?:string; icon_url?:string;
    installed_version?:string;
  };
  type ContentResult = {
    project_id:string; title:string; description:string;
    icon_url?:string; downloads:number; version_number?:string; installed?:boolean;
  };
  type ModVersion = {
    id: string;
    version_number: string;
    game_versions: string[];
    loaders: string[];
    date_published: string;
  };
  type E4allConfig = {
    minecraft_dir:string;
    use_broker:boolean; broker_url:string; relay_host:string; relay_port:number;
    restore_dedicated_commands:boolean; use_whitelist:boolean; host_enabled:boolean;
    prevent_pause_while_hosting:boolean;
    dialtone_host_enabled:boolean; dialtone_player_enabled:boolean;
    dialtone_relay_map:string; dialtone_sanitize_ticket:boolean;
    reconnect_max_attempts:number; reconnect_base_delay_seconds:number;
    keepalive_interval_seconds:number; login_ready_timeout_ms:number;
    relay_connect_timeout_ms:number;
    offline_mode:boolean; hide_domain_in_chat:boolean; voice_p2p_enabled:boolean;
  };
  type E4allStatus = {
    dir_exists:boolean; loader:string; loader_id:string; loader_source:string;
    installed:boolean; jar:string; config_exists:boolean; config_path:string;
    level:string; message:string;
  };
  type LauncherCandidate = {
    label: string; path: string; exists: boolean;
    loader: string; e4all_installed: boolean; config_exists: boolean;
  };
  type ServerProperties = {
    motd:string; max_players:number; gamemode:string; difficulty:string;
    white_list:boolean; online_mode:boolean; allow_flight:boolean; force_gamemode:boolean;
    spawn_protection:number; require_resource_pack:boolean; resource_pack:string;
    resource_pack_prompt:string; view_distance:number; simulation_distance:number;
    pvp:boolean; spawn_monsters:boolean; spawn_animals:boolean; generate_structures:boolean;
    hardcore:boolean; enable_command_block:boolean; allow_nether:boolean;
    level_name:string; level_seed:string; level_type:string; server_port:number;
    icon_exists:boolean; timezone:string; java_version:string;
  };
  type Server = {
    status:string; address:string; players:number; memory_mb:number;
    memory_limit_gb:number; cpu_percent:number; uptime_seconds:number;
    server_name:string; version:string; server_dir:string; java_path:string;
    java_detected:boolean; installed:boolean; e4all:E4allConfig; logs:Log[];
  };
  type ServerSummary = { server_name:string; version:string; server_dir:string; installed:boolean; active:boolean; status:string };

  const E4_DEFAULTS: Omit<E4allConfig, 'minecraft_dir'> = {
    use_broker:true, broker_url:'https://broker.e4mc.link/getBestRelay', relay_host:'test.e4mc.link', relay_port:25575,
    restore_dedicated_commands:true, use_whitelist:false, host_enabled:true, prevent_pause_while_hosting:true,
    dialtone_host_enabled:true, dialtone_player_enabled:true, dialtone_relay_map:'https://natives.e4mc.link/relaymap.json', dialtone_sanitize_ticket:true,
    reconnect_max_attempts:5, reconnect_base_delay_seconds:2, keepalive_interval_seconds:15,
    login_ready_timeout_ms:15000, relay_connect_timeout_ms:15000, offline_mode:false, hide_domain_in_chat:false, voice_p2p_enabled:true,
  };

  let server:Server = {
    status:'offline', address:'localhost:25565', players:0, memory_mb:0,
    memory_limit_gb:4, cpu_percent:0, uptime_seconds:0,
    server_name:'Meu servidor', version:'',
    server_dir:'~/Minecraft/MeuServidor', java_path:'java',
    java_detected:false, installed:false,
    e4all:{ minecraft_dir:'~/.minecraft', ...E4_DEFAULTS },
    logs:[]
  };
  let versions:Version[] = [];
  let serverProfiles:ServerSummary[] = [];
  let switchingServer = false;
  let activeTab:'overview'|'content'|'config'|'setup' = 'overview';

  let contentKind = 'mods';
  let contentLoader = 'fabric';
  let contentVersion = '';
  let content:ContentItem[] = [];
  let results:ContentResult[] = [];

  let setup = {
    server_name:'Meu Servidor Minecraft',
    server_dir:'~/Minecraft/Servidor',
    memory_limit_gb:4,
    loader:'vanilla',
  };
  let setupVersion = '';

  let loading = false;
  let notice = '';
  let saved = false;
  let command = '';
  let booted = false;

  let selectedItems: Set<string> = new Set();

  let deleteOpen = false;
  let deleteStep: 1 | 2 = 1;
  let deleteName = '';
  let deleteAck = false;
  let deleting = false;

  let catalogOpen = false;
  let catalogDetail:ContentResult | null = null;
  let catalogLoading = false;
  let catalogError = '';
  let catalogQuery = '';
  let catalogCategory = 'all';
  let showCatalogFilters = false;
  let catalogOffset = 0;
  let catalogHasMore = true;
  let catalogLoadingMore = false;
  let catalogSentinel:HTMLDivElement | undefined;
  let catalogScroll:HTMLDivElement | undefined;
  let catalogObserver:IntersectionObserver | undefined;
  let consoleEl:HTMLDivElement | undefined;
  let consolePinned = true;
  let iconInput:HTMLInputElement | undefined;

  let catalogSelected: Set<string> = new Set();
  let modVersions: ModVersion[] = [];
  let selectedModVersion = '';
  let selectedModVersions:Record<string,string> = {};
  let installingBatch = false;

  let e4status: E4allStatus | null = null;
  let checkingE4 = false;

  let launcherCandidates: LauncherCandidate[] = [];
  let scanningLaunchers = false;

  let props: ServerProperties = {
    motd:'Servidor ShadowPainel', max_players:20, gamemode:'survival', difficulty:'easy',
    white_list:false, online_mode:true, allow_flight:false, force_gamemode:false,
    spawn_protection:0, require_resource_pack:false, resource_pack:'', resource_pack_prompt:'',
    view_distance:10, simulation_distance:10, pvp:true, spawn_monsters:true,
    spawn_animals:true, generate_structures:true, hardcore:false, enable_command_block:false,
    allow_nether:true, level_name:'world', level_seed:'', level_type:'minecraft:normal',
    server_port:25565, icon_exists:false, timezone:'America/Sao_Paulo', java_version:'Java 21',
  };
  let loadingProps = false;
  let savedProps = false;

  function handleConsoleScroll() {
    if (!consoleEl) return;
    consolePinned = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 48;
  }
  afterUpdate(() => {
    if (consoleEl && consolePinned) consoleEl.scrollTop = consoleEl.scrollHeight;
  });

  async function uploadServerIcon(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    if (file.type !== 'image/png') { notice = 'O ícone precisa ser um arquivo PNG.'; return; }
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(new Error('Falha ao ler o ícone.'));
      reader.readAsDataURL(file);
    });
    try {
      const result = await api<{message?:string}>('/server/icon', json({data_url:dataUrl}));
      notice = result.message || 'Ícone salvo. Reinicie o servidor para aplicar.';
      await loadServerProperties();
    } catch (e) {
      notice = e instanceof Error ? e.message : 'Falha ao salvar o ícone.';
    } finally {
      input.value = '';
    }
  }

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

  async function refresh() {
    try {
      const fresh = await api<Server>('/state');
      server = fresh;
      if (booted === false) {
        booted = true;
        if (!fresh.installed) activeTab = 'setup';
      }
      if (!contentVersion && fresh.version) contentVersion = fresh.version;
    } catch {
      notice = 'Núcleo Rust offline ou ainda iniciando.';
    }
  }

  async function loadServers() {
    try {
      const data = await api<{servers:ServerSummary[]}>('/servers');
      serverProfiles = data.servers || [];
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Não foi possível carregar os servidores.';
    }
  }

  async function switchServer(server_dir:string) {
    if (!server_dir || server_dir === server.server_dir) return;
    switchingServer = true;
    try {
      server = await api<Server>('/servers/switch', json({server_dir}));
      contentVersion = server.version || contentVersion;
      activeTab = server.installed ? 'overview' : 'setup';
      notice = `Servidor ativo: ${server.server_name}.`;
      await loadServers();
      await loadContent();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Não foi possível trocar de servidor.';
      await loadServers();
    }
    switchingServer = false;
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
      selectedItems.clear();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao listar conteúdo.';
    }
  }

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
      await loadServers();
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

  $: if (activeTab === 'config') loadServerProperties();

  async function loadServerProperties() {
    try {
      loadingProps = true;
      const data = await api<ServerProperties>('/server/properties');
      if (data) props = { ...props, ...data };
    } catch (e) {
      console.error('Falha ao carregar server.properties', e);
    } finally {
      loadingProps = false;
    }
  }

  async function saveServerProperties() {
    try {
      loadingProps = true;
      await api('/server/properties', json(props));
      savedProps = true;
      setTimeout(() => savedProps = false, 2200);
    } catch (e) {
      notice = e instanceof Error ? e.message : 'Falha ao salvar server.properties.';
    } finally {
      loadingProps = false;
    }
  }

  async function installContent(project_id:string, version_id?: string) {
    try {
      const version = contentVersion || server.version;
      await api('/content/install', json({
        project_id, kind:contentKind, loader:contentLoader, version,
        version_id,
      }));
      notice = 'Conteúdo instalado.';
      await loadContent();
      await runCatalogSearch();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao instalar conteúdo.';
    }
  }

  async function installBatch() {
    if (catalogSelected.size === 0) return;
    installingBatch = true;
    notice = '';
    let success = 0;
    let errors: string[] = [];

    for (const projectId of catalogSelected) {
      try {
        const version = contentVersion || server.version;
        const versionId = selectedModVersions[projectId] || undefined;
        await api('/content/install', json({
          project_id: projectId, kind: contentKind, loader: contentLoader, version,
          version_id: versionId,
        }));
        success++;
      } catch (e) {
        errors.push(`${projectId}: ${e instanceof Error ? e.message : 'erro'}`);
      }
    }

    installingBatch = false;
    if (errors.length > 0) {
      notice = `${success} instalado(s). Erros: ${errors.join(', ')}`;
    } else {
      notice = `${success} mod(s) instalado(s) com sucesso!`;
      catalogSelected.clear();
    }
    await loadContent();
    await runCatalogSearch();
  }

  async function deleteSelectedContent() {
    if (selectedItems.size === 0) return;
    if (!confirm(`Excluir ${selectedItems.size} arquivo(s) selecionado(s)?`)) return;
    try {
      const result = await api<{ok:boolean; deleted?:string[]; errors?:string[]}>('/content/delete', json({
        kind: contentKind,
        filenames: Array.from(selectedItems)
      }));
      if (result.ok) {
        notice = `${result.deleted?.length || 0} arquivo(s) removido(s).`;
      } else {
        notice = `Erros: ${result.errors?.join(', ')}`;
      }
      await loadContent();
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao excluir.';
    }
  }

  function toggleSelection(filename: string) {
    if (selectedItems.has(filename)) selectedItems.delete(filename);
    else selectedItems.add(filename);
    selectedItems = selectedItems;
  }

  function selectAll() {
    if (selectedItems.size === content.length) selectedItems.clear();
    else content.forEach(c => selectedItems.add(c.filename));
    selectedItems = selectedItems;
  }

  async function openCatalog() {
    catalogOpen = true;
    catalogDetail = null;
    catalogSelected.clear();
    selectedModVersions = {};
    catalogOffset = 0;
    catalogHasMore = true;
    await runCatalogSearch();
  }
  async function runCatalogSearch(loadMore = false) {
    if (catalogLoading || catalogLoadingMore || (loadMore && !catalogHasMore)) return;
    if (loadMore) catalogLoadingMore = true;
    else { catalogLoading = true; catalogOffset = 0; catalogHasMore = true; }
    catalogError = '';
    try {
      const version = contentVersion || server.version;
      const data = await api<{hits:ContentResult[]}>('/content/search', json({
        query: catalogQuery,
        kind: contentKind,
        loader: contentLoader,
        version,
        category: catalogCategory,
        offset: catalogOffset,
        limit: 12
      }));
      const hits = data.hits || [];
      results = loadMore ? [...results, ...hits] : hits;
      catalogOffset += hits.length;
      catalogHasMore = hits.length === 12;
    } catch (e) {
      catalogError = e instanceof Error ? e.message : 'Falha na busca.';
    }
    catalogLoading = false;
    catalogLoadingMore = false;
  }
  function observeCatalog(node:HTMLDivElement) {
    catalogSentinel = node;
    catalogObserver?.disconnect();
    catalogObserver = new IntersectionObserver(entries => {
      if (entries[0]?.isIntersecting) runCatalogSearch(true);
    }, {root: catalogScroll, rootMargin:'260px'});
    catalogObserver.observe(node);
    return { destroy() { catalogObserver?.disconnect(); } };
  }
  function pickCategory(kind:'mods'|'plugins'|'datapacks') {
    contentKind = kind;
    loadContent();
    if (catalogOpen) runCatalogSearch();
  }

  async function loadModVersions(projectId: string) {
    modVersions = [];
    selectedModVersion = '';
    if (!projectId) return;
    try {
      const version = contentVersion || server.version;
      const data = await api<ModVersion[]>(`/content/versions?query=${projectId}&loader=${contentLoader}&version=${version}`);
      modVersions = data;
      if (modVersions.length > 0) {
        selectedModVersion = selectedModVersions[projectId] || modVersions[0].id;
        selectedModVersions = {...selectedModVersions, [projectId]:selectedModVersion};
      }
    } catch (e) {
      console.error(e);
    }
  }

  function isInstalled(projectId: string) {
    return results.some(item => item.project_id === projectId && item.installed === true);
  }

  async function toggleCatalogSelection(item: ContentResult) {
    if (isInstalled(item.project_id)) return;
    if (!selectedModVersions[item.project_id]) await loadModVersions(item.project_id);
    if (catalogSelected.has(item.project_id)) catalogSelected.delete(item.project_id);
    else catalogSelected.add(item.project_id);
    catalogSelected = catalogSelected;
  }

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
      await loadServers();
    } catch (e) {
      notice = e instanceof Error ? e.message : 'Falha ao excluir servidor.';
    }
    deleting = false;
  }

  async function checkE4all() {
    checkingE4 = true;
    try {
      e4status = await api<E4allStatus>('/e4all/status', json({ minecraft_dir: server.e4all.minecraft_dir }));
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Não foi possível verificar o e4all.';
    }
    checkingE4 = false;
  }
  async function loadE4allFromFile() {
    try {
      server.e4all = await api<E4allConfig>('/e4all/load', json({ minecraft_dir: server.e4all.minecraft_dir }));
      await checkE4all();
      saved = true; setTimeout(()=>saved=false, 2200);
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao ler o e4all.toml';
    }
  }
  async function saveE4all() {
    notice = '';
    try {
      server = await api<Server>('/e4all/config', json(server.e4all));
      await checkE4all();
      saved = true; setTimeout(()=>saved=false, 2200);
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao salvar e4all';
    }
  }
  function resetE4allDefaults() {
    server.e4all = { minecraft_dir: server.e4all.minecraft_dir, ...E4_DEFAULTS };
  }
  function loaderName(l:string) {
    return ({ fabric:'Fabric', forge:'Forge', neoforge:'NeoForge', quilt:'Quilt', none:'Nenhum (sem mod loader)' } as Record<string,string>)[l] ?? l;
  }

  async function scanLaunchers() {
    scanningLaunchers = true;
    try {
      launcherCandidates = await api<LauncherCandidate[]>('/e4all/scan');
      if (launcherCandidates.length === 0) notice = 'Nenhum launcher encontrado automaticamente.';
    } catch(e) {
      notice = e instanceof Error ? e.message : 'Falha ao procurar launchers.';
    } finally {
      scanningLaunchers = false;
    }
  }

  function pickLauncherPath(path: string) {
    server.e4all.minecraft_dir = path;
    checkE4all();
  }

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

  $: if (activeTab === 'content') loadContent();
  $: if (contentKind || contentVersion || contentLoader) results = [];
  $: if (activeTab === 'config' && !e4status) checkE4all();

  onMount(() => {
    refresh();
    loadServers();
    loadVersions();
    const timer = setInterval(() => { refresh(); loadServers(); }, 3000);
    return ()=>{ clearInterval(timer); catalogObserver?.disconnect(); };
  });
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="mark"><img src="/shadowpainel.png" alt="" /></div>
      <div><strong>Shadow<span>Painel</span></strong><small>SERVER CONTROL</small></div>
    </div>
    <div class="workspace">
      <div class="server-avatar">☾</div>
      <div class="workspace-picker">
        <small class="workspace-caption">SERVIDOR ATIVO</small>
        <select aria-label="Selecionar servidor" value={server.server_dir} disabled={switchingServer || server.status !== 'offline'} on:change={(e)=>switchServer((e.currentTarget as HTMLSelectElement).value)}>
          {#if serverProfiles.length === 0}<option value={server.server_dir}>{server.server_name}</option>{/if}
          {#each serverProfiles as profile}
            <option value={profile.server_dir}>{profile.server_name} · {profile.version || 'sem versão'}{profile.status === 'online' ? ' · online' : ''}</option>
          {/each}
        </select>
        <small>{serverProfiles.length} servidor(es) cadastrado(s)</small>
      </div>
      <button class="dots" title="Atualizar lista" on:click={loadServers}>↻</button>
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
            <div class="console" bind:this={consoleEl} on:scroll={handleConsoleScroll}>
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
          <div style="display:flex; gap:10px;">
            <button class="secondary" on:click={selectAll}>
              {selectedItems.size === content.length ? 'Desmarcar Todos' : 'Selecionar Todos'}
            </button>
            <button class="primary" on:click={openCatalog}>
              ＋ Adicionar {contentKind === 'mods' ? 'mods' : contentKind === 'plugins' ? 'plugins' : 'datapacks'}
            </button>
          </div>
        </div>

        <div class="content-tabs">
          {#each ([['mods','Mods'],['plugins','Plugins'],['datapacks','Datapacks']] as [string,string][]) as item}
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
              <div class="installed-card"
                   class:selected={selectedItems.has(file.filename)}
                   role="button" tabindex="0"
                   on:dblclick={() => toggleSelection(file.filename)}
                   on:keydown={(event) => (event.key === 'Enter' || event.key === ' ') && toggleSelection(file.filename)}>
                <input type="checkbox" class="item-checkbox"
                       checked={selectedItems.has(file.filename)}
                       on:change={() => toggleSelection(file.filename)} />
                <div class="catalog-icon" style="width:36px; height:36px; font-size:14px;">
                  {#if file.icon_url}
                    <img src={file.icon_url} alt=""/>
                  {:else}
                    <span>{contentKind==='mods'?'◆':contentKind==='plugins'?'⬡':'◇'}</span>
                  {/if}
                </div>
                <div class="installed-meta">
                  <b title={file.title || file.filename}>{file.title || file.filename}</b>
                  <small>{#if file.installed_version}Versão: {file.installed_version} · {/if}{formatBytes(file.size)} · {file.filename}</small>
                </div>
              </div>
            {/each}
          </div>

          {#if selectedItems.size > 0}
            <div class="bulk-actions">
              <button class="danger" on:click={deleteSelectedContent}>
                Remover {selectedItems.size} arquivo(s) selecionado(s)
              </button>
            </div>
          {/if}
        {/if}
      </section>

      {#if catalogOpen}
        <div class="modal-overlay" role="presentation" on:click|self={()=>{catalogOpen=false;catalogDetail=null;}} on:keydown={(e)=>e.key==='Escape'&&(catalogOpen=false,catalogDetail=null)}>
          <div class="modal catalog-modal">
            <div class="modal-head">
              <div>
                <h3>Adicionar {contentKind}</h3>
                <p>Explore por nome, autor ou descrição. Filtros: {contentVersion || '—'} · {contentLoader}</p>
              </div>
              <button class="modal-close" on:click={()=>{catalogOpen=false;catalogDetail=null;}}>✕</button>
            </div>

            <div class="catalog-search-container">
              <div class="catalog-search">
                <input bind:value={catalogQuery}
                       placeholder="Pesquisar no Modrinth (ex.: sodium, worldedit)..."
                       on:keydown={(e)=>e.key==='Enter'&&runCatalogSearch()}/>
                <button class="secondary" on:click={() => showCatalogFilters = !showCatalogFilters} title="Filtros Avançados">⚙ Filtros</button>
                <button class="primary" on:click={() => runCatalogSearch()} disabled={catalogLoading || !contentVersion}>
                  {catalogLoading?'Buscando...':'Buscar'}
                </button>
              </div>

              {#if showCatalogFilters}
                <div class="catalog-advanced-filters">
                  <label>
                    Categoria
                    <select bind:value={catalogCategory} on:change={() => runCatalogSearch()}>
                      <option value="all">Todas as categorias</option>
                      <option value="adventure">Aventura</option>
                      <option value="exploration">Exploração</option>
                      <option value="worldgen">Geração de Mundo</option>
                      <option value="magic">Magia</option>
                      <option value="technology">Tecnologia</option>
                      <option value="decoration">Decoração</option>
                      <option value="equipment">Equipamentos</option>
                      <option value="optimization">Otimização</option>
                      <option value="utility">Utilidades</option>
                      <option value="library">Biblioteca (APIs)</option>
                    </select>
                  </label>
                </div>
              {/if}
            </div>

            {#if catalogError}<div class="notice">{catalogError}</div>{/if}

            {#if catalogLoading}
              <div class="catalog-loading"><div class="spinner"></div> Consultando Modrinth…</div>
            {:else if results.length===0}
              <div class="empty">Nenhum resultado para <b>{contentVersion}</b> em <b>{contentLoader}</b>.</div>
            {:else}
              <div class="catalog-grid" bind:this={catalogScroll}>
                {#each results as item}
                  <div class="catalog-card"
                       class:selected={catalogSelected.has(item.project_id)}
                       class:installed={isInstalled(item.project_id)}
                       role="button" tabindex="0"
                       on:click={() => !isInstalled(item.project_id) && (catalogDetail=item, loadModVersions(item.project_id))}
                       on:dblclick|stopPropagation={() => toggleCatalogSelection(item)}
                       on:keydown={(e)=>(e.key==='Enter'||e.key===' ')&&!isInstalled(item.project_id)&&(catalogDetail=item, loadModVersions(item.project_id))}>
                    {#if !isInstalled(item.project_id)}
                      <input type="checkbox" class="item-checkbox"
                             checked={catalogSelected.has(item.project_id)}
                             on:click|stopPropagation={() => toggleCatalogSelection(item)} />
                    {/if}
                    <div class="catalog-icon">
                      {#if item.icon_url}<img src={item.icon_url} alt="" loading="lazy"/>
                      {:else}<span>{item.title.slice(0,1).toUpperCase()}</span>{/if}
                    </div>
                    <div class="catalog-text">
                      <b>{item.title}</b>
                      <small>{item.description || 'Projeto Modrinth'}</small>
                      <span class="catalog-downloads">⇩ {item.downloads.toLocaleString('pt-BR')}</span>
                    </div>
                    {#if isInstalled(item.project_id)}
                      <span class="installed-badge">Já instalado</span>
                    {/if}
                  </div>
                {/each}
                {#if catalogHasMore}<div class="catalog-sentinel" use:observeCatalog>{catalogLoadingMore ? 'Carregando mais resultados…' : 'Role para carregar mais'}</div>{/if}
              </div>
            {/if}

            {#if catalogDetail}
              <div class="detail-sheet">
                <div class="detail-head">
                  <div class="catalog-icon big">
                    {#if catalogDetail.icon_url}<img src={catalogDetail.icon_url} alt=""/>
                    {:else}<span>{catalogDetail.title.slice(0,1).toUpperCase()}</span>{/if}
                  </div>
                  <div>
                    <h4>{catalogDetail.title}</h4>
                    {#if catalogDetail.version_number}<small class="version-tag">Versão: {catalogDetail.version_number}</small>{/if}
                    <small>{catalogDetail.downloads.toLocaleString('pt-BR')} downloads</small>
                  </div>
                  <button class="modal-close" on:click={()=>{catalogDetail=null; modVersions=[];}}>✕</button>
                </div>
                <p>{catalogDetail.description || 'Sem descrição fornecida.'}</p>

                {#if modVersions.length > 0}
                  <div class="version-picker">
                    <label for="content-version-select">Selecionar versão do mod</label>
                    <select id="content-version-select" bind:value={selectedModVersion} on:change={()=>catalogDetail && (selectedModVersions={...selectedModVersions,[catalogDetail.project_id]:selectedModVersion})}>
                      {#each modVersions as v}
                        <option value={v.id}>{v.version_number} ({v.game_versions.join(', ')})</option>
                      {/each}
                    </select>
                  </div>
                {/if}

                <div class="detail-actions">
                  <button class="secondary" on:click={()=>{catalogDetail=null; modVersions=[];}}>Cancelar</button>
                  <button class="primary"
                          on:click={async () => {
                            if (catalogDetail) {
                              await installContent(catalogDetail.project_id, selectedModVersion);
                              catalogDetail=null;
                              modVersions=[];
                            }
                          }}>
                    Confirmar Instalação
                  </button>
                </div>
              </div>
            {/if}

            <div class="modal-footer-batch">
              <span>{catalogSelected.size} mod(s) selecionado(s)</span>
              <button class="primary" on:click={installBatch} disabled={catalogSelected.size === 0 || installingBatch}>
                {installingBatch ? 'Instalando...' : 'Instalar Selecionados'}
              </button>
            </div>
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
        <div class="card-head" style="padding-bottom: 16px;">
          <div><h3 style="display:flex; align-items:center; gap:8px;"><span style="font-size:20px; color:var(--accent);">📄</span> server.properties</h3></div>
          <div style="display:flex; gap:10px; align-items:center;">
            {#if savedProps}<span style="color:var(--green); font-size:12px; font-family:'DM Mono';">✓ Salvo</span>{/if}
            <button class="secondary" on:click={saveServerProperties} disabled={loadingProps}>
              {loadingProps ? 'Salvando...' : 'Salvar Alterações'}
            </button>
          </div>
        </div>

        <div class="props-grid">
          <div class="prop-group" style="grid-column: 1 / 2;">
            <span class="prop-label">MOTD (Mensagem do dia)</span>
            <div class="prop-control"><input bind:value={props.motd} placeholder="Servidor ShadowPainel" /></div>
            <small>motd=Servidor ShadowPainel</small>
          </div>
          <div class="prop-group" style="grid-column: 2 / 3;">
            <span class="prop-label">Vagas</span>
            <div class="prop-control"><input type="number" bind:value={props.max_players} min="1" max="1000" /></div>
            <small>max-players=20</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Modo de jogo</span>
            <div class="prop-control">
              <select bind:value={props.gamemode}>
                <option value="survival">Sobrevivência</option>
                <option value="creative">Criativo</option>
                <option value="adventure">Aventura</option>
                <option value="spectator">Espectador</option>
              </select>
            </div>
            <small>gamemode=survival</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Dificuldade</span>
            <div class="prop-control">
              <select bind:value={props.difficulty}>
                <option value="peaceful">Pacífico</option>
                <option value="easy">Fácil</option>
                <option value="normal">Normal</option>
                <option value="hard">Difícil</option>
              </select>
            </div>
            <small>difficulty=easy</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Whitelist</span>
            <div class="prop-control toggle-control">
              <button class="toggle-btn" class:active={props.white_list} on:click={() => props.white_list = !props.white_list}>
                {#if props.white_list}✓{:else}✕{/if}
              </button>
            </div>
            <small>white-list=false</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Pirata (Offline Mode)</span>
            <div class="prop-control toggle-control">
              <button class="toggle-btn" class:active={!props.online_mode} on:click={() => props.online_mode = !props.online_mode}>
                {#if !props.online_mode}✓{:else}✕{/if}
              </button>
            </div>
            <small>online-mode=true</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Voar</span>
            <div class="prop-control toggle-control">
              <button class="toggle-btn" class:active={props.allow_flight} on:click={() => props.allow_flight = !props.allow_flight}>
                {#if props.allow_flight}✓{:else}✕{/if}
              </button>
            </div>
            <small>allow-flight=true</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Forçar o Gamemode</span>
            <div class="prop-control toggle-control">
              <button class="toggle-btn" class:active={props.force_gamemode} on:click={() => props.force_gamemode = !props.force_gamemode}>
                {#if props.force_gamemode}✓{:else}✕{/if}
              </button>
            </div>
            <small>force-gamemode=false</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">Proteção no Spawn</span>
            <div class="prop-control">
              <div class="number-input">
                <span>🛡️</span>
                <input type="number" bind:value={props.spawn_protection} min="0" max="1000" style="text-align:center; border:none; background:transparent;" />
                <div class="num-buttons">
                  <button on:click={() => props.spawn_protection++}>+</button>
                  <button on:click={() => props.spawn_protection = Math.max(0, props.spawn_protection - 1)}>-</button>
                </div>
              </div>
            </div>
            <small>spawn-protection=0</small>
          </div>
          <div class="prop-group">
            <span class="prop-label">É necessário um pacote de recursos</span>
            <div class="prop-control toggle-control">
              <button class="toggle-btn" class:active={props.require_resource_pack} on:click={() => props.require_resource_pack = !props.require_resource_pack}>
                {#if props.require_resource_pack}✓{:else}✕{/if}
              </button>
            </div>
            <small>require-resource-pack=false</small>
          </div>
          <div class="prop-group" style="grid-column: 1 / -1;">
            <span class="prop-label">Pacote de recursos</span>
            <div class="prop-control"><input bind:value={props.resource_pack} placeholder="https://exemplo.com/pacote.zip" /></div>
            <small>resource-pack=</small>
          </div>
          <div class="prop-group" style="grid-column: 1 / -1;">
            <span class="prop-label">Solicitação do pacote de recursos</span>
            <div class="prop-control"><input bind:value={props.resource_pack_prompt} placeholder="Você precisa baixar o pacote para jogar!" /></div>
            <small>resource-pack-prompt=</small>
          </div>
        </div>

        <div class="settings-subhead"><h4>Mundo, desempenho e rede</h4><p>Essas opções são gravadas no server.properties e normalmente exigem reiniciar o servidor.</p></div>
        <div class="props-grid">
          <div class="prop-group"><span class="prop-label">Nome do mundo</span><div class="prop-control"><input bind:value={props.level_name} /></div><small>level-name</small></div>
          <div class="prop-group"><span class="prop-label">Seed</span><div class="prop-control"><input bind:value={props.level_seed} placeholder="vazio = aleatória" /></div><small>level-seed</small></div>
          <div class="prop-group"><span class="prop-label">Tipo de mundo</span><div class="prop-control"><select bind:value={props.level_type}><option value="minecraft:normal">Normal</option><option value="minecraft:flat">Plano</option><option value="minecraft:amplified">Amplificado</option><option value="minecraft:large_biomes">Biomas grandes</option></select></div><small>level-type</small></div>
          <div class="prop-group"><span class="prop-label">Porta do servidor</span><div class="prop-control"><input type="number" min="1" max="65535" bind:value={props.server_port} /></div><small>server-port</small></div>
          <div class="prop-group"><span class="prop-label">Distância de visão</span><div class="prop-control"><input type="number" min="3" max="32" bind:value={props.view_distance} /></div><small>view-distance</small></div>
          <div class="prop-group"><span class="prop-label">Distância de simulação</span><div class="prop-control"><input type="number" min="3" max="32" bind:value={props.simulation_distance} /></div><small>simulation-distance</small></div>
          <div class="prop-group"><span class="prop-label">PvP</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.pvp} on:click={() => props.pvp = !props.pvp}>{#if props.pvp}✓{:else}✕{/if}</button></div><small>pvp</small></div>
          <div class="prop-group"><span class="prop-label">Comandos de bloco</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.enable_command_block} on:click={() => props.enable_command_block = !props.enable_command_block}>{#if props.enable_command_block}✓{:else}✕{/if}</button></div><small>enable-command-block</small></div>
          <div class="prop-group"><span class="prop-label">Monstros</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.spawn_monsters} on:click={() => props.spawn_monsters = !props.spawn_monsters}>{#if props.spawn_monsters}✓{:else}✕{/if}</button></div><small>spawn-monsters</small></div>
          <div class="prop-group"><span class="prop-label">Animais</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.spawn_animals} on:click={() => props.spawn_animals = !props.spawn_animals}>{#if props.spawn_animals}✓{:else}✕{/if}</button></div><small>spawn-animals</small></div>
          <div class="prop-group"><span class="prop-label">Estruturas</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.generate_structures} on:click={() => props.generate_structures = !props.generate_structures}>{#if props.generate_structures}✓{:else}✕{/if}</button></div><small>generate-structures</small></div>
          <div class="prop-group"><span class="prop-label">Nether</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.allow_nether} on:click={() => props.allow_nether = !props.allow_nether}>{#if props.allow_nether}✓{:else}✕{/if}</button></div><small>allow-nether</small></div>
          <div class="prop-group"><span class="prop-label">Hardcore</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={props.hardcore} on:click={() => props.hardcore = !props.hardcore}>{#if props.hardcore}✓{:else}✕{/if}</button></div><small>hardcore</small></div>
        </div>
        <div class="settings-subhead"><h4>Ícone do servidor</h4><p>PNG estático de 64×64 pixels salvo como server-icon.png na pasta do servidor.</p></div>
        <div class="icon-upload-row">
          <span class:ok={props.icon_exists} class="icon-status">{props.icon_exists ? '✓ Ícone instalado' : '○ Nenhum ícone instalado'}</span>
          <input bind:this={iconInput} type="file" accept="image/png" on:change={uploadServerIcon} />
        </div>
        <div style="display:grid; grid-template-columns: 1fr 1fr; gap:20px; margin-top:24px; padding-top:20px; border-top:1px dashed var(--border);">
          <div class="prop-group">
            <span class="prop-label">Fuso horário</span>
            <div class="prop-control">
              <select bind:value={props.timezone}>
                <option value="America/Sao_Paulo">America/Sao_Paulo</option>
                <option value="America/New_York">America/New_York</option>
                <option value="Europe/London">Europe/London</option>
                <option value="UTC">UTC</option>
              </select>
            </div>
          </div>
          <div class="prop-group">
            <span class="prop-label">Ambiente</span>
            <div class="prop-control">
              <select bind:value={props.java_version}>
                <option value="Java 17">Java 17</option>
                <option value="Java 21">Java 21</option>
                <option value="Java 25">Java 25</option>
              </select>
            </div>
          </div>
        </div>
      </section>

      <section class="settings-card">
        <div class="card-head">
          <div><h3>e4all</h3><p>Configuração do mod para abrir um mundo LAN pela internet.</p></div>
          <span class="mod-badge">MODRINTH · e4all</span>
        </div>

        <div class="e4-folder" style="display:grid;grid-template-columns:1fr auto auto;gap:10px;margin-bottom:16px;">
          <label style="display:flex;flex-direction:column;gap:8px;color:var(--muted);font:500 10px 'DM Mono';letter-spacing:.7px;">
            Pasta do Minecraft
            <input bind:value={server.e4all.minecraft_dir} placeholder="~/.minecraft"
                   style="background:var(--bg);border:1px solid var(--border);border-radius:6px;padding:11px;color:var(--text);font:12px 'DM Mono';outline:none"/>
          </label>
          <button class="secondary" on:click={checkE4all} disabled={checkingE4}>
            {checkingE4 ? 'Verificando...' : 'Verificar'}
          </button>
          <button class="secondary" on:click={loadE4allFromFile}>Ler do arquivo</button>
        </div>

        <div style="display:flex;gap:10px;margin-bottom:14px;flex-wrap:wrap; margin-top: 16px;">
          <button class="secondary" on:click={scanLaunchers} disabled={scanningLaunchers}>
            {scanningLaunchers ? 'Procurando...' : '🔍 Procurar launchers'}
          </button>
        </div>

        {#if launcherCandidates.length > 0}
          <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:10px;margin-bottom:20px;">
            {#each launcherCandidates as c}
              <button
                style="text-align:left;background:var(--panel-2);border:1px solid var(--border);border-radius:8px;padding:12px;cursor:pointer;color:var(--text);transition:0.2s;"
                on:mouseenter={(e) => (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--accent)'}
                on:mouseleave={(e) => (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--border)'}
                on:click={() => pickLauncherPath(c.path)}>
                <b style="display:block;font-size:12px;color:#fff;">{c.label}</b>
                <small style="display:block;color:var(--muted);font-size:10px;margin-top:4px;word-break:break-all;">{c.path}</small>
                <small style="display:block;margin-top:8px;font-family:'DM Mono';font-size:10px;">
                  loader: <span class:yes={c.loader !== 'none' && c.loader !== 'quilt'} class:no={c.loader === 'none'} class:maybe={c.loader === 'quilt'}>{c.loader}</span>
                  · e4all: <span class:yes={c.e4all_installed} class:no={!c.e4all_installed}>{c.e4all_installed ? 'Sim' : 'Não'}</span>
                  · toml: <span class:yes={c.config_exists} class:maybe={!c.config_exists}>{c.config_exists ? 'Sim' : 'Não'}</span>
                </small>
              </button>
            {/each}
          </div>
        {/if}

        {#if e4status}
          {#if e4status.dir_exists}
            <div class="e4-rows" style="display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin-bottom:16px;">
              <div class="e4-row" style="background:var(--panel-2);border-radius:7px;padding:12px 14px;">
                <span style="display:block;color:var(--faint);font:500 9px 'DM Mono';">MOD LOADER</span>
                <b class:yes={e4status.loader!=='none' && e4status.loader!=='quilt'} class:no={e4status.loader==='none'} class:maybe={e4status.loader==='quilt'} style="display:block;margin-top:6px;font-size:13px;">{loaderName(e4status.loader)}</b>
                <small style="display:block;color:var(--faint);font-size:9px;margin-top:4px;">{e4status.loader_id ? e4status.loader_id + ' · ' : ''}{e4status.loader_source}</small>
              </div>
              <div class="e4-row" style="background:var(--panel-2);border-radius:7px;padding:12px 14px;">
                <span style="display:block;color:var(--faint);font:500 9px 'DM Mono';">e4all INSTALADO</span>
                <b class:yes={e4status.installed} class:no={!e4status.installed} style="display:block;margin-top:6px;font-size:13px;">{e4status.installed ? 'Sim' : 'Não'}</b>
                <small style="display:block;color:var(--faint);font-size:9px;margin-top:4px;">{e4status.installed ? e4status.jar : 'Nenhum JAR do e4all na pasta mods'}</small>
              </div>
              <div class="e4-row" style="background:var(--panel-2);border-radius:7px;padding:12px 14px;">
                <span style="display:block;color:var(--faint);font:500 9px 'DM Mono';">ARQUIVO e4all.toml</span>
                <b class:yes={e4status.config_exists} class:maybe={!e4status.config_exists} style="display:block;margin-top:6px;font-size:13px;">{e4status.config_exists ? 'Encontrado' : 'Será criado ao salvar'}</b>
                <small style="display:block;color:var(--faint);font-size:9px;margin-top:4px;word-break:break-all;">{e4status.config_path}</small>
              </div>
            </div>
          {/if}
          <div class="notice" style="margin:0 0 16px;">{e4status.message}</div>
        {/if}

        <div class="settings-subhead"><h4>Hospedagem</h4></div>
        <div class="e4all-options">
          <div class="prop-group"><span class="prop-label">Compartilhar mundos LAN</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.host_enabled} on:click={() => server.e4all.host_enabled = !server.e4all.host_enabled}>{#if server.e4all.host_enabled}✓{:else}✕{/if}</button></div><small>hostEnabled</small></div>
          <div class="prop-group"><span class="prop-label">Não pausar enquanto hospeda</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.prevent_pause_while_hosting} on:click={() => server.e4all.prevent_pause_while_hosting = !server.e4all.prevent_pause_while_hosting}>{#if server.e4all.prevent_pause_while_hosting}✓{:else}✕{/if}</button></div><small>preventPauseWhileHosting</small></div>
          <div class="prop-group"><span class="prop-label">Esconder domínio no chat</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.hide_domain_in_chat} on:click={() => server.e4all.hide_domain_in_chat = !server.e4all.hide_domain_in_chat}>{#if server.e4all.hide_domain_in_chat}✓{:else}✕{/if}</button></div><small>hideDomainInChat</small></div>
        </div>

        <div class="settings-subhead"><h4>Acesso</h4></div>
        <div class="e4all-options">
          <div class="prop-group"><span class="prop-label">Contas offline</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.offline_mode} on:click={() => server.e4all.offline_mode = !server.e4all.offline_mode}>{#if server.e4all.offline_mode}✓{:else}✕{/if}</button></div><small>offlineMode</small></div>
          <div class="prop-group"><span class="prop-label">Usar whitelist</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.use_whitelist} on:click={() => server.e4all.use_whitelist = !server.e4all.use_whitelist}>{#if server.e4all.use_whitelist}✓{:else}✕{/if}</button></div><small>useWhiteList</small></div>
          <div class="prop-group"><span class="prop-label">Comandos de servidor dedicado</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.restore_dedicated_commands} on:click={() => server.e4all.restore_dedicated_commands = !server.e4all.restore_dedicated_commands}>{#if server.e4all.restore_dedicated_commands}✓{:else}✕{/if}</button></div><small>restoreDedicatedCommands</small></div>
        </div>

        <div class="settings-subhead"><h4>Voz e P2P</h4></div>
        <div class="e4all-options">
          <div class="prop-group"><span class="prop-label">Simple Voice Chat P2P</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.voice_p2p_enabled} on:click={() => server.e4all.voice_p2p_enabled = !server.e4all.voice_p2p_enabled}>{#if server.e4all.voice_p2p_enabled}✓{:else}✕{/if}</button></div><small>voiceP2PEnabled</small></div>
          <div class="prop-group"><span class="prop-label">P2P como host</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.dialtone_host_enabled} on:click={() => server.e4all.dialtone_host_enabled = !server.e4all.dialtone_host_enabled}>{#if server.e4all.dialtone_host_enabled}✓{:else}✕{/if}</button></div><small>dialtoneHostEnabled</small></div>
          <div class="prop-group"><span class="prop-label">P2P como jogador</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.dialtone_player_enabled} on:click={() => server.e4all.dialtone_player_enabled = !server.e4all.dialtone_player_enabled}>{#if server.e4all.dialtone_player_enabled}✓{:else}✕{/if}</button></div><small>dialtonePlayerEnabled</small></div>
          <div class="prop-group"><span class="prop-label">Ocultar IP direto do relay</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.dialtone_sanitize_ticket} on:click={() => server.e4all.dialtone_sanitize_ticket = !server.e4all.dialtone_sanitize_ticket}>{#if server.e4all.dialtone_sanitize_ticket}✓{:else}✕{/if}</button></div><small>dialtoneSanitizeTicket</small></div>
        </div>

        <div class="settings-subhead"><h4>Relay</h4></div>
        <div class="e4all-options">
          <div class="prop-group"><span class="prop-label">Escolher melhor relay</span><div class="prop-control toggle-control"><button class="toggle-btn" class:active={server.e4all.use_broker} on:click={() => server.e4all.use_broker = !server.e4all.use_broker}>{#if server.e4all.use_broker}✓{:else}✕{/if}</button></div><small>useBroker</small></div>
        </div>
        <div class="form-grid">
          <label>URL do broker<input bind:value={server.e4all.broker_url}></label>
          <label>URL do mapa de relays (P2P)<input bind:value={server.e4all.dialtone_relay_map}></label>
          <label>Relay fixo (host)<input bind:value={server.e4all.relay_host}></label>
          <label>Relay fixo (porta)<input type="number" min="1" max="65535" bind:value={server.e4all.relay_port}></label>
        </div>

        <div class="settings-subhead"><h4>Avançado · reconexão e tempos</h4></div>
        <div class="form-grid">
          <label>Tentativas de reconexão<input type="number" min="0" max="100" bind:value={server.e4all.reconnect_max_attempts}></label>
          <label>Atraso inicial (s)<input type="number" min="1" max="3600" bind:value={server.e4all.reconnect_base_delay_seconds}></label>
          <label>Keepalive (s)<input type="number" min="1" max="3600" bind:value={server.e4all.keepalive_interval_seconds}></label>
          <label>Espera do login (ms)<input type="number" min="0" max="600000" bind:value={server.e4all.login_ready_timeout_ms}></label>
          <label>Tempo de conexão ao relay (ms)<input type="number" min="100" max="600000" bind:value={server.e4all.relay_connect_timeout_ms}></label>
        </div>

        <div class="e4all-note" style="margin-top:16px;">
          <b>Como aplicar</b>
          <p>O e4all é um mod do cliente: só quem hospeda instala. O domínio é gerado pelo mod ao abrir o mundo em "Abrir para LAN". Salve com o Minecraft fechado e reabra o mundo LAN depois. No Android, use o JAR com sufixo <b>-android</b>; o P2P de voz pode não funcionar em redes com NAT restritivo.</p>
        </div>

        <div style="display:flex;gap:10px;margin-top:16px;">
          <button class="primary" on:click={saveE4all}>SALVAR NO e4all.toml <span>→</span></button>
          <button class="secondary" on:click={resetE4allDefaults}>Restaurar padrões</button>
        </div>
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
            <small style="color: var(--faint); font-size: 11px; margin-top: 4px;">Este nome aparece no painel. Pode ser alterado depois.</small>
          </label>
          <label>
            Pasta do servidor
            <input bind:value={setup.server_dir}/>
            <small style="color: var(--faint); font-size: 11px; margin-top: 4px;">Onde os arquivos do mundo serão salvos. Use o caminho completo.</small>
          </label>
          <label>
            Versão do Minecraft
            <select bind:value={setupVersion}>
              {#each versions as version}
                <option value={version.id}>{version.id}</option>
              {/each}
            </select>
          </label>
          <label>
            Tipo / modloader
            <select bind:value={setup.loader}>
              <option value="vanilla">Vanilla — servidor oficial (recomendado para iniciantes)</option>
              <option value="fabric">Fabric — para usar mods</option>
              <option value="paper">Paper — para usar plugins (melhor desempenho)</option>
              <option value="purpur">Purpur — plugins + customizações</option>
            </select>
          </label>
          <label>Memória inicial (GB)
            <input type="number" min="1" max="64" bind:value={setup.memory_limit_gb}/>
          </label>
        </div>

        <div class="setup-note">
          <b>Depois da instalação</b>
          <span>Você poderá administrar mods, plugins e datapacks na área de conteúdo, com busca por versão no Modrinth.</span>
        </div>

        <button class="primary" on:click={createServer} disabled={loading || !setupVersion}>
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
      <div class="modal-overlay" role="presentation" on:click|self={closeDelete} on:keydown={(e)=>e.key==='Escape'&&closeDelete()}>
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
              <span><input type="checkbox" bind:checked={deleteAck}/>Entendo que os arquivos serão apagados permanentemente.</span>
              <small>Não é possível desfazer.</small>
            </label>
            <div class="modal-actions">
              <button class="secondary" on:click={closeDelete}>Cancelar</button>
              <button class="danger" disabled={!deleteAck} on:click={()=>deleteStep=2}>Continuar</button>
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