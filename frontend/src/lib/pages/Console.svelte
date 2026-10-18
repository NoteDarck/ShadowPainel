<script lang="ts">
  import { serverStore } from '../stores/servers';
  let command = $state('');
  const logs = [
    { time: '10:42:01', text: 'Boot sequence iniciado', kind: 'command' },
    { time: '10:42:05', text: 'Loading 42 plugins…', kind: '' },
    { time: '10:42:11', text: 'Done! Listening on :25565', kind: 'success' },
  ];
</script>

<header>
  <div>
    <div class="breadcrumb">painel <span>/</span> consola</div>
    <h1>Consola</h1>
    <p>{serverStore.active?.name ?? '—'}</p>
  </div>
</header>

<div class="console-card" style="margin: 0 40px 40px;">
  <div class="card-head">
    <div><h3>Saída</h3><p>Tempo real</p></div>
    <div class="live"><span></span>LIVE</div>
  </div>
  <div class="console" style="min-height: 420px;">
    {#each logs as l}
      <div class="log">
        <span class="time">{l.time}</span>
        <span class={l.kind}>{l.text}</span>
      </div>
    {/each}
  </div>
  <form class="command-line" onsubmit={(e) => { e.preventDefault(); command = ''; }}>
    <span>&gt;</span>
    <input placeholder="Comando…" bind:value={command} />
    <button type="submit">Enviar ↵</button>
  </form>
</div>