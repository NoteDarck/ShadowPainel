<script lang="ts">
  import { onMount } from 'svelte';
  const APP_VERSION = '0.7.0';
  type Log = { time:string; text:string; kind:string };
  type Version = { id:string; kind:string; release_time:string };
  type ContentItem = { filename:string; size:number; kind:string };
  type ContentResult = { project_id:string; title:string; description:string; icon_url?:string; downloads:number };
  type E4all = { loader:string; minecraft_version:string; android_variant:boolean; offline_accounts:boolean; voice_chat:boolean; install_path:string; public_address:string };
  type Server = { status:string; address:string; players:number; memory_mb:number; memory_limit_gb:number; cpu_percent:number; uptime_seconds:number; server_name:string; version:string; server_dir:string; java_path:string; java_detected:boolean; e4all:E4all; logs:Log[] };

  let server:Server = { status:'offline', address:'localhost:25565', players:0, memory_mb:0, memory_limit_gb:4, cpu_percent:0, uptime_seconds:0, server_name:'Meu servidor', version:'', server_dir:'~/Minecraft/MeuServidor', java_path:'java', java_detected:false, e4all:{loader:'fabric',minecraft_version:'',android_variant:false,offline_accounts:false,voice_chat:true,install_path:'~/Minecraft/MeuServidor/mods',public_address:''}, logs:[] };
  let versions:Version[] = [];
  let activeTab = 'overview';
  let contentKind = 'mods';
  let contentLoader = 'fabric';
  let selectedVersion = '';
  let content:ContentItem[] = [];
  let results:ContentResult[] = [];
  let query = '';
  let loading = false;
  let searching = false;
  let notice = '';
  let saved = false;
  let setupOpen = false;
  let setup = { server_name:'Meu servidor', version:'', loader:'vanilla', server_dir:'~/Minecraft/MeuServidor', memory_limit_gb:4 };
  let command = '';

  const api = async <T=unknown>(path:string, options?:RequestInit):Promise<T> => {
    const response = await fetch(`/api${path}`, options);
    const text = await response.text();
    let data:any = {};
    try { data = text ? JSON.parse(text) : {}; } catch { throw new Error(`Resposta inválida (${response.status})`); }
    if (!response.ok) throw new Error(typeof data.error === 'string' ? data.error : `Erro ${response.status}`);
    return data as T;
  };
  const json = (value:unknown):RequestInit => ({method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(value)});

  async function refresh() { try { server = await api<Server>('/state'); if (server.version) { selectedVersion = server.version; setup.version = server.version; } if (server.e4all?.loader) contentLoader = server.e4all.loader; } catch { notice = 'Núcleo Rust offline ou ainda iniciando.'; } }
  async function loadVersions() { try { const data = await api<{versions:Version[]}>('/minecraft/versions'); versions = data.versions.filter(v => v.kind === 'release'); if (!selectedVersion && versions.length) selectedVersion = server.version || versions[0].id; if (!setup.version) setup.version = selectedVersion; } catch(e) { notice = e instanceof Error ? e.message : 'Não foi possível ler versões.'; } }
  async function loadContent() { try { const data = await api<{items:ContentItem[]}>(`/content?kind=${contentKind}`); content = data.items; } catch(e) { notice = e instanceof Error ? e.message : 'Falha ao listar conteúdo.'; } }
  async function createServer() { loading=true; notice=''; try { server=await api<Server>('/server/create', json({...setup, version:selectedVersion})); setupOpen=false; activeTab='overview'; notice='Servidor instalado. Aceite a EULA antes de iniciar.'; await loadContent(); } catch(e) { notice=e instanceof Error?e.message:'Falha na instalação.'; } loading=false; }
  async function saveConfig() { try { server=await api<Server>('/config', json({server_name:server.server_name, version:server.version, server_dir:server.server_dir, memory_limit_gb:server.memory_limit_gb})); saved=true; setTimeout(()=>saved=false,2200); } catch(e) { notice=e instanceof Error?e.message:'Falha ao salvar.'; } }
  async function toggleServer() { loading=true; try { server=await api<Server>(server.status==='online'?'/server/stop':'/server/start',{method:'POST'}); } catch(e) { notice=e instanceof Error?e.message:'Falha ao controlar servidor.'; } loading=false; }
  async function sendCommand() { if(!command.trim()) return; try { await api('/command',json({command})); command=''; await refresh(); } catch(e) { notice=e instanceof Error?e.message:'Console indisponível.'; } }
  async function searchContent() { if(!query.trim()) return; searching=true; try { const data=await api<{hits:ContentResult[]}>('/content/search',json({query,kind:contentKind,loader:contentLoader,version:selectedVersion || server.version})); results=data.hits || []; } catch(e) { notice=e instanceof Error?e.message:'Falha na busca.'; } searching=false; }
  async function installContent(project_id:string) { try { await api('/content/install',json({project_id,kind:contentKind,loader:contentLoader,version:selectedVersion || server.version})); notice='Conteúdo instalado.'; await loadContent(); } catch(e) { notice=e instanceof Error?e.message:'Falha ao instalar conteúdo.'; } }
  async function deleteContent(filename:string) { if(!confirm(`Excluir ${filename}?`)) return; try { await api('/content/delete',json({kind:contentKind,filename})); await loadContent(); } catch(e) { notice=e instanceof Error?e.message:'Falha ao excluir.'; } }
  function formatBytes(n:number) { return n < 1024*1024 ? `${Math.max(1,Math.round(n/1024))} KB` : `${(n/1024/1024).toFixed(1)} MB`; }
  function uptime(n:number) { const h=Math.floor(n/3600).toString().padStart(2,'0'),m=Math.floor(n%3600/60).toString().padStart(2,'0'),s=Math.floor(n%60).toString().padStart(2,'0'); return `${h}:${m}:${s}`; }
  $: if (activeTab === 'content') loadContent();
  $: if (contentKind || selectedVersion || contentLoader) results=[];
  onMount(() => { refresh(); loadVersions(); const timer=setInterval(refresh,3000); return ()=>clearInterval(timer); });
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand"><div class="mark">S</div><div><strong>Shadow<span>Painel</span></strong><small>SERVER CONTROL</small></div></div>
    <div class="workspace"><div class="server-avatar">☾</div><div><b>{server.server_name}</b><small>{server.version || 'NOVA INSTALAÇÃO'}</small></div><button class="dots">•••</button></div>
    <nav>
      <div class="nav-label">SERVIDOR</div>
      <button class:active={activeTab==='overview'} on:click={()=>activeTab='overview'}><span class="nav-icon">◈</span>Visão geral</button>
      <button class:active={activeTab==='content'} on:click={()=>{activeTab='content';loadContent()}}><span class="nav-icon">▦</span>Mods, plugins e datapacks</button>
      <button class:active={activeTab==='setup'} on:click={()=>setupOpen=true}><span class="nav-icon">＋</span>Criar servidor</button>
      <div class="nav-label settings-label">SISTEMA</div>
      <button class:active={activeTab==='config'} on:click={()=>activeTab='config'}><span class="nav-icon">⚙</span>Configurações avançadas</button>
    </nav>
    <div class="sidebar-footer"><div class="secure"><span>●</span><div><b>Execução local</b><small>Seus dados ficam no PC</small></div></div><div class="user"><div class="avatar">NP</div><div><b>Administrador</b><small>ShadowPainel v{APP_VERSION}</small></div></div></div>
  </aside>

  <main class="main">
    <header><div><div class="breadcrumb">SERVIDORES <span>/</span> {server.server_name}</div><h1>{activeTab==='content'?'Conteúdo do servidor':activeTab==='config'?'Configurações avançadas':activeTab==='setup'?'Criar servidor':'Olá, administrador'} <span class="wave">✦</span></h1><p>{activeTab==='content'?'Organize seus arquivos como uma instância do Prism Launcher.':activeTab==='config'?'A instalação inicial fica separada das preferências do servidor.':'Gerencie seu mundo sem abrir o terminal.'}</p></div><div class="header-actions"><button class="icon-button">◔</button><button class="help">? <span>Ajuda</span></button></div></header>
    {#if notice}<div class="notice">{notice}</div>{/if}{#if saved}<div class="notice success-notice">Configuração salva.</div>{/if}

    {#if activeTab==='overview'}
      <div class="hero"><div class="hero-copy"><div class="eyebrow"><span class:online={server.status==='online'} class="status-dot"></span>{server.status==='online'?'SERVIDOR ONLINE':'SERVIDOR OFFLINE'}</div><h2>{server.server_name}</h2><p>Minecraft Java Edition · {server.version || 'sem versão instalada'}</p></div><div class="orb"><div class="orb-inner">☾</div><i></i><i></i></div><div class="hero-actions"><button class="primary" on:click={toggleServer} disabled={loading}>{loading?'CARREGANDO...':server.status==='online'?'PARAR SERVIDOR':'INICIAR SERVIDOR'} <span>→</span></button><button class="secondary" on:click={()=>setupOpen=true}>＋ Nova instalação</button></div><div class="hero-meta"><div><span>ENDEREÇO</span><b>{server.address}</b></div><div class="divider"></div><div><span>UPTIME</span><b>{uptime(server.uptime_seconds)}</b></div></div></div>
      <section class="metrics"><div class="metric"><div class="metric-icon blue">♙</div><div><span>JOGADORES ONLINE</span><strong>{server.players}</strong></div></div><div class="metric"><div class="metric-icon purple">▣</div><div><span>MEMÓRIA UTILIZADA</span><strong>{server.memory_mb} <em>/ {server.memory_limit_gb*1024} MB</em></strong></div></div><div class="metric"><div class="metric-icon green">◔</div><div><span>USO DE CPU</span><strong>{server.cpu_percent.toFixed(1)}<em>%</em></strong></div></div></section>
      <div class="content-grid"><section class="console-card"><div class="card-head"><div><h3>Console do servidor</h3><p>Logs em tempo real</p></div><div class="live"><span></span> AO VIVO</div></div><div class="console">{#each server.logs as log}<div class="log"><span class="time">{log.time}</span><span>{log.text}</span></div>{/each}</div><div class="command-line"><span>›</span><input bind:value={command} on:keydown={(e)=>e.key==='Enter'&&sendCommand()} placeholder="Digite um comando..."/><button on:click={sendCommand}>Enviar ↗</button></div></section><section class="quick-card"><div class="card-head"><div><h3>Ações rápidas</h3><p>Atalhos de administração</p></div></div><button class="quick" on:click={toggleServer} disabled={loading}><span class="q-icon">{server.status==='online'?'■':'▶'}</span><div><b>{server.status==='online'?'Parar':'Iniciar'} servidor</b><small>Controle seguro do processo Java</small></div><span>›</span></button><button class="quick" on:click={()=>{activeTab='content';loadContent()}}><span class="q-icon">▦</span><div><b>Gerenciar conteúdo</b><small>Mods, plugins e datapacks</small></div><span>›</span></button><button class="quick" on:click={()=>activeTab='config'}><span class="q-icon">⚙</span><div><b>Configurações avançadas</b><small>Memória, pasta e e4all</small></div><span>›</span></button></section></div>

    {:else if activeTab==='content'}
      <section class="settings-card content-manager"><div class="card-head"><div><h3>Arquivos da instância</h3><p>Separe o conteúdo por categoria e veja o que está instalado antes de alterar o servidor.</p></div><span class="mod-badge">PRISM STYLE</span></div><div class="content-tabs">{#each [['mods','Mods'],['plugins','Plugins'],['datapacks','Datapacks']] as item}<button class:active={contentKind===item[0]} on:click={()=>{contentKind=item[0];loadContent()}}>{item[1]}</button>{/each}</div><div class="content-filters"><label>Versão Minecraft<select bind:value={selectedVersion}>{#each versions as version}<option value={version.id}>{version.id}</option>{/each}</select></label><label>Modloader<select bind:value={contentLoader}><option value="fabric">Fabric</option><option value="forge">Forge</option><option value="neoforge">NeoForge</option><option value="quilt">Quilt</option><option value="paper">Paper</option><option value="purpur">Purpur</option><option value="vanilla">Vanilla</option></select></label><label class="filter-category">Categoria<select bind:value={contentKind}><option value="mods">Mods</option><option value="plugins">Plugins</option><option value="datapacks">Datapacks</option></select></label></div><div class="content-toolbar"><input bind:value={query} placeholder={`Pesquisar ${contentKind} no Modrinth...`} on:keydown={(e)=>e.key==='Enter'&&searchContent()}/><button class="primary" on:click={searchContent} disabled={searching || !selectedVersion}>{searching?'Buscando...':'Buscar e adicionar'}</button></div><div class="content-columns"><div><h4>Instalados <span>{content.length}</span></h4>{#if content.length===0}<div class="empty">Nenhum arquivo em <b>{contentKind}/</b>.</div>{:else}<div class="file-list">{#each content as file}<div class="file-row"><span class="file-icon">{contentKind==='mods'?'◆':contentKind==='plugins'?'⬡':'◇'}</span><div><b>{file.filename}</b><small>{formatBytes(file.size)} · {contentKind}</small></div><button class="danger" on:click={()=>deleteContent(file.filename)}>Excluir</button></div>{/each}</div>{/if}</div><div><h4>Resultados compatíveis</h4>{#if results.length===0}<div class="empty">Pesquise para encontrar {contentKind} compatíveis com Minecraft {selectedVersion || 'selecione uma versão'} e {contentLoader}.</div>{:else}<div class="file-list">{#each results as item}<div class="file-row"><div>{#if item.icon_url}<img class="project-icon" src={item.icon_url} alt=""/>{/if}</div><div><b>{item.title}</b><small>{item.description || 'Projeto Modrinth'} · {item.downloads} downloads</small></div><button class="secondary small-button" on:click={()=>installContent(item.project_id)}>Adicionar</button></div>{/each}</div>{/if}</div></div></section>

    {:else if activeTab==='config'}
      <section class="settings-card"><div class="card-head"><div><h3>Configurações avançadas</h3><p>Estas opções ficam separadas do assistente de instalação inicial.</p></div></div><div class="form-grid"><label>Nome do servidor<input bind:value={server.server_name}/></label><label>Versão atual<input bind:value={server.version}/></label><label>Pasta do servidor<input bind:value={server.server_dir}/></label><label>Memória (GB)<input type="number" min="1" max="64" bind:value={server.memory_limit_gb}/></label></div><div class="java-check"><span class:ok={server.java_detected}>●</span><div><b>{server.java_detected?'Java detectado':'Java não detectado'}</b><small>{server.java_path} · necessário para iniciar</small></div></div><button class="primary" on:click={saveConfig}>SALVAR CONFIGURAÇÃO <span>→</span></button></section>
      <section class="settings-card"><div class="card-head"><div><h3>e4all</h3><p>Configurações complementares do servidor.</p></div></div><div class="form-grid"><label>Loader<select bind:value={server.e4all.loader}><option value="fabric">Fabric</option><option value="forge">Forge</option><option value="neoforge">NeoForge</option><option value="quilt">Quilt</option></select></label><label>Versão Minecraft<input bind:value={server.e4all.minecraft_version}/></label><label>Pasta de instalação<input bind:value={server.e4all.install_path}/></label><label>Endereço público<input bind:value={server.e4all.public_address}/></label></div><button class="primary" on:click={async()=>{try { server=await api<Server>('/e4all/config',json(server.e4all));saved=true;setTimeout(()=>saved=false,2200);}catch(e){notice=e instanceof Error?e.message:'Falha ao salvar.';}}}>SALVAR e4all <span>→</span></button></section>

    {:else if activeTab==='setup' || setupOpen}
      <section class="settings-card setup-card"><div class="card-head"><div><h3>Instalação inicial do servidor</h3><p>Escolha a versão do Minecraft, o tipo de servidor e a pasta. O ShadowPainel baixa e prepara os arquivos automaticamente.</p></div><span class="mod-badge">ASSISTENTE</span></div><div class="setup-steps"><span class="current">1. Identidade</span><span>2. Versão e loader</span><span>3. Conteúdo filtrado por versão</span></div><div class="form-grid"><label>Nome do servidor<input bind:value={setup.server_name} placeholder="Sobrevivência principal"/></label><label>Pasta do servidor<input bind:value={setup.server_dir}/></label><label>Versão do Minecraft<select bind:value={selectedVersion}>{#each versions as version}<option value={version.id}>{version.id}</option>{/each}</select><small class="selected-version">Selecionada: {selectedVersion || 'carregando versões...'}</small></label><label>Tipo / modloader<select bind:value={setup.loader}><option value="vanilla">Vanilla — server.jar oficial</option><option value="fabric">Fabric — mods</option><option value="paper">Paper — plugins</option><option value="purpur">Purpur — plugins</option></select></label><label>Memória inicial (GB)<input type="number" min="1" max="64" bind:value={setup.memory_limit_gb}/></label></div><div class="setup-note"><b>Depois da instalação</b><span>Você poderá administrar mods, plugins e datapacks na área de conteúdo, com busca por versão no Modrinth.</span></div><button class="primary" on:click={createServer} disabled={loading || !setup.version}>{loading?'INSTALANDO...':'INSTALAR E ABRIR PAINEL'} <span>→</span></button></section>
    {/if}
    <footer><span>SHADOWPAINEL <b>v{APP_VERSION}</b></span><span>RUST CORE · MULTIPLATAFORMA <i>●</i></span></footer>
  </main>
</div>
