<script lang="ts">
  import { serverStore } from '../stores/servers';
  import type { Server, ServerStatus } from '../stores/servers';

  let query = $state('');

  const filtered = $derived(
    serverStore.list.filter(s =>
      s.name.toLowerCase().includes(query.toLowerCase()) ||
      s.type.toLowerCase().includes(query.toLowerCase())
    )
  );

  function statusLabel(s: ServerStatus) {
    return { online: 'Online', offline: 'Offline', starting: 'A iniciar…',
             stopping: 'A parar…', error: 'Erro' }[s];
  }
</script>

<section class="servers-page">
  <div class="page-head">
    <div>
      <h1>Todos os servidores <span class="count">{serverStore.list.length}</span></h1>
      <p>Gere todos os teus servidores num só lugar.</p>
    </div>
    <a class="primary" href="#/servers/new">+ Criar servidor</a>
  </div>

  <input class="search" placeholder="Pesquisar por nome ou tipo…" bind:value={query} />

  <div class="server-grid">
    {#each filtered as s (s.id)}
      <article class="server-card" class:active={s.id === serverStore.activeId}>
        <header>
          <div class="s-icon">{s.type[0]}</div>
          <div class="s-title">
            <b>{s.name}</b>
            <small>{s.type} {s.version} · Java {s.javaVersion}</small>
          </div>
          <span class="pill {s.status}">{statusLabel(s.status)}</span>
        </header>

        <div class="s-meta">
          <span>👥 {s.players}/{s.maxPlayers}</span>
          <span>💾 {s.ram}</span>
          <span>🔌 :{s.port}</span>
        </div>

        <footer>
          <button class="secondary small"
                  onclick={() => serverStore.select(s.id)}
                  disabled={s.id === serverStore.activeId}>
            {s.id === serverStore.activeId ? 'Selecionado' : 'Selecionar'}
          </button>
          <button class="danger-outline small" onclick={() => serverStore.remove(s.id)}>
            Eliminar
          </button>
        </footer>
      </article>
    {:else}
      <div class="empty">
        <b>Sem resultados</b> — nenhum servidor corresponde à pesquisa.
      </div>
    {/each}
  </div>
</section>

<style>
  .servers-page { padding: 32px 40px 40px; }
  .page-head { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 20px; }
  .page-head h1 { font: 600 28px Outfit, sans-serif; color: #fff; }
  .count { background: rgba(141,93,255,.18); color: var(--accent-2);
           border-radius: 12px; padding: 2px 10px; font: 12px 'DM Mono'; margin-left: 8px; }
  .page-head p { color: var(--muted); font-size: 13px; margin-top: 4px; }
  .search { width: 100%; max-width: 420px; background: var(--bg); border: 1px solid var(--border);
            border-radius: 8px; padding: 12px 14px; color: var(--text);
            font: 13px Inter, sans-serif; margin-bottom: 24px; outline: none; }
  .search:focus { border-color: var(--accent); box-shadow: 0 0 0 2px rgba(141,93,255,.1); }
  .server-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 16px; }
  .server-card { background: var(--panel); border: 1px solid var(--border); border-radius: 12px;
                 padding: 18px; display: flex; flex-direction: column; gap: 14px;
                 transition: border-color .2s; }
  .server-card:hover { border-color: var(--muted); }
  .server-card.active { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  .server-card header { display: flex; align-items: center; gap: 12px; }
  .s-icon { width: 40px; height: 40px; border-radius: 8px; background: rgba(141,93,255,.12);
            color: var(--accent-2); display: grid; place-items: center;
            font: 700 16px Outfit, sans-serif; flex-shrink: 0; }
  .s-title { flex: 1; min-width: 0; }
  .s-title b { display: block; color: #fff; font-size: 14px; }
  .s-title small { display: block; color: var(--muted); font-size: 11px; margin-top: 2px; }
  .pill { font: 10px 'DM Mono'; padding: 4px 8px; border-radius: 6px; white-space: nowrap; }
  .pill.online { background: rgba(89,203,138,.15); color: var(--green); }
  .pill.offline { background: rgba(141,150,170,.15); color: var(--muted); }
  .pill.starting, .pill.stopping { background: rgba(217,155,87,.15); color: var(--yellow); }
  .pill.error { background: rgba(255,92,92,.15); color: var(--red); }
  .s-meta { display: flex; gap: 16px; font: 11px 'DM Mono'; color: var(--muted); }
  .server-card footer { display: flex; gap: 8px; }
  .small { padding: 8px 12px !important; font-size: 11px !important; }
  .empty { grid-column: 1 / -1; min-height: 120px; border: 1px dashed var(--border);
           border-radius: 8px; display: grid; place-items: center;
           color: var(--muted); font-size: 13px; }
</style>