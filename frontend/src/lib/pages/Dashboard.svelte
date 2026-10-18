<script lang="ts">
  import { serverStore, serverActions, loadingStore } from '../stores/servers';

  let command = '';

  function handleSendCommand() {
    serverActions.sendCommand(command);
    command = '';
  }

  function uptime(n: number) {
    const h = Math.floor(n / 3600).toString().padStart(2, '0'),
          m = Math.floor((n % 3600) / 60).toString().padStart(2, '0'),
          s = Math.floor(n % 60).toString().padStart(2, '0');
    return `${h}:${m}:${s}`;
  }
</script>

{#if !$serverStore.installed}
  <section class="settings-card setup-card">
    <div class="card-head">
      <div>
        <h3>Servidor não instalado</h3>
        <p>Esta instalação ainda não foi concluída ou a pasta está vazia.</p>
      </div>
    </div>
    <button class="primary" on:click={() => window.location.hash = '#/setup'}>
      ABRIR ASSISTENTE <span>→</span>
    </button>
  </section>
{:else}
  <div class="hero">
    <div class="hero-copy">
      <div class="eyebrow">
        <span class:online={$serverStore.status === 'online'} class="status-dot"></span>
        {$serverStore.status === 'online' ? 'SERVIDOR ONLINE' : 'SERVIDOR OFFLINE'}
      </div>
      <h2>{$serverStore.server_name}</h2>
      <p>Minecraft Java Edition · {$serverStore.version}</p>
    </div>
    <div class="orb"><div class="orb-inner">☾</div><i></i><i></i></div>
    
    <div class="hero-bottom">
      <div class="hero-actions">
        <button class="primary" on:click={serverActions.toggleServer} disabled={$loadingStore}>
          {$loadingStore ? 'CARREGANDO...' : $serverStore.status === 'online' ? 'PARAR SERVIDOR' : 'INICIAR SERVIDOR'} <span>→</span>
        </button>
        <button class="secondary" on:click={serverActions.acceptEula}>ACEITAR EULA</button>
      </div>
      <div class="hero-meta">
        <div><span>ENDEREÇO</span><b>{$serverStore.address}</b></div>
        <div class="divider"></div>
        <div><span>UPTIME</span><b>{uptime($serverStore.uptime_seconds)}</b></div>
      </div>
    </div>
  </div>

  <section class="metrics">
    <div class="metric">
      <div class="metric-icon blue">♙</div>
      <div><span>JOGADORES ONLINE</span><strong>{$serverStore.players}</strong></div>
    </div>
    <div class="metric">
      <div class="metric-icon purple">▣</div>
      <div><span>MEMÓRIA UTILIZADA</span>
        <strong>{$serverStore.memory_mb} <em>/ {$serverStore.memory_limit_gb * 1024} MB</em></strong>
      </div>
    </div>
    <div class="metric">
      <div class="metric-icon green">◔</div>
      <div><span>USO DE CPU</span><strong>{$serverStore.cpu_percent.toFixed(1)}<em>%</em></strong></div>
    </div>
  </section>

  <div class="content-grid">
    <section class="console-card">
      <div class="card-head">
        <div><h3>Console do servidor</h3><p>Logs em tempo real</p></div>
        <div class="live"><span></span> AO VIVO</div>
      </div>
      <div class="console">
        {#each $serverStore.logs as log}
          <div class="log">
            <span class="time">{log.time}</span>
            <span class={log.kind}>{log.text}</span>
          </div>
        {/each}
      </div>
      <div class="command-line">
        <span>›</span>
        <input bind:value={command} on:keydown={(e) => e.key === 'Enter' && handleSendCommand()} placeholder="Digite um comando..."/>
        <button on:click={handleSendCommand}>Enviar ↗</button>
      </div>
    </section>
    
    <section class="quick-card">
      <div class="card-head">
        <div><h3>Ações rápidas</h3><p>Atalhos de administração</p></div>
      </div>
      <button class="quick" on:click={serverActions.toggleServer} disabled={$loadingStore}>
        <span class="q-icon">{$serverStore.status === 'online' ? '■' : '▶'}</span>
        <div><b>{$serverStore.status === 'online' ? 'Parar' : 'Iniciar'} servidor</b>
          <small>Controle seguro do processo Java</small></div>
        <span>›</span>
      </button>
      <button class="quick" on:click={() => window.location.hash = '#/content'}>
        <span class="q-icon">▦</span>
        <div><b>Gerenciar conteúdo</b><small>Mods, plugins e datapacks</small></div>
        <span>›</span>
      </button>
      <button class="quick" on:click={() => window.location.hash = '#/settings'}>
        <span class="q-icon">⚙</span>
        <div><b>Configurações</b><small>Memória, pasta e e4all</small></div>
        <span>›</span>
      </button>
    </section>
  </div>
{/if}