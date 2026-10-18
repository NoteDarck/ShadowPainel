<script lang="ts">
  import { serverStore, serverActions } from './stores/servers';

  function handleSwitch(id: string) {
    if ($serverStore.active_server_id === id) return;
    serverActions.switchServer(id);
  }
</script>

<div class="server-selector">
  <div class="nav-label" style="padding: 0 24px 8px;">SEUS SERVIDORES</div>
  <div class="server-list">
    {#if $serverStore.servers.length === 0}
      <div style="padding: 0 24px; color: var(--faint); font-size: 11px;">Nenhum servidor.</div>
    {/if}
    {#each $serverStore.servers as srv}
      <button 
        class="server-item" 
        class:active={$serverStore.active_server_id === srv.id}
        on:click={() => handleSwitch(srv.id)}>
        <div class="server-avatar">☾</div>
        <div>
          <b>{srv.server_name}</b>
          <small>{srv.version}</small>
        </div>
        {#if $serverStore.active_server_id === srv.id}
          <span class="active-dot" class:online={$serverStore.status === 'online'}></span>
        {/if}
      </button>
    {/each}
  </div>
  <button class="new-server-btn" on:click={() => window.location.hash = '#/setup'}>
    <span>＋</span> Novo servidor
  </button>
</div>

<style>
  /* Seletor de Servidores */
  .server-selector { display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px; }
  .server-list { display: flex; flex-direction: column; max-height: 220px; overflow-y: auto; padding: 0 12px; }
  
  .server-item {
    display: flex; align-items: center; gap: 12px;
    padding: 10px 12px; background: none; border: 1px solid transparent;
    border-radius: 8px; cursor: pointer; text-align: left; transition: 0.2s; color: var(--text); width: 100%;
  }
  .server-item:hover { background: var(--panel-2); }
  .server-item.active { background: rgba(141, 93, 255, 0.1); border-color: rgba(141, 93, 255, 0.2); }
  
  .server-avatar {
    width: 32px; height: 32px; background: #1f2436; border-radius: 6px;
    display: grid; place-items: center; color: var(--accent); font-size: 16px; flex-shrink: 0;
  }
  .server-item.active .server-avatar { background: var(--accent); color: #fff; }
  
  .server-item > div:nth-child(2) { flex: 1; min-width: 0; }
  .server-item b { display: block; font-size: 12px; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .server-item small { display: block; font-family: 'DM Mono', monospace; font-size: 9px; color: var(--muted); margin-top: 2px; }
  
  .active-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--red); box-shadow: 0 0 8px var(--red); }
  .active-dot.online { background: var(--green); box-shadow: 0 0 8px var(--green); }
  
  .new-server-btn {
    display: flex; align-items: center; gap: 8px; margin: 8px 16px 0; padding: 10px 14px;
    background: var(--panel-2); border: 1px dashed var(--border); border-radius: 8px;
    color: var(--muted); font-size: 12px; font-weight: 500; cursor: pointer; transition: 0.2s;
  }
  .new-server-btn:hover { border-color: var(--accent); color: var(--accent-2); background: rgba(141, 93, 255, 0.05); }
</style>