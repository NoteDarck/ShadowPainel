<script lang="ts">
  import { serverStore } from '../stores/servers';
  let tab = $state<'installed' | 'catalog'>('installed');
  const installed = [
    { name: 'EssentialsX', version: '2.20.1', file: 'EssentialsX.jar' },
    { name: 'LuckPerms', version: '5.4.130', file: 'LuckPerms.jar' },
    { name: 'Vault', version: '1.7.3', file: 'Vault.jar' },
  ];
</script>

<header>
  <div>
    <div class="breadcrumb">painel <span>/</span> conteúdo</div>
    <h1>Conteúdo</h1>
    <p>{serverStore.active?.name ?? '—'} · {serverStore.active?.type} {serverStore.active?.version}</p>
  </div>
</header>

<div class="settings-card content-manager">
  <div class="content-tabs">
    <button class:active={tab === 'installed'} onclick={() => tab = 'installed'}>Instalados</button>
    <button class:active={tab === 'catalog'}   onclick={() => tab = 'catalog'}>Catálogo</button>
  </div>

  {#if tab === 'installed'}
    <div class="content-columns">
      <div>
        <h4>Plugins <span>{installed.length}</span></h4>
        <div class="file-list">
          {#each installed as p}
            <div class="file-row">
              <div class="file-icon">📦</div>
              <div><b>{p.name}</b><small>{p.version} · {p.file}</small></div>
              <button class="danger small-button">Remover</button>
            </div>
          {/each}
        </div>
      </div>
      <div>
        <h4>Mods <span>0</span></h4>
        <div class="empty"><b>Sem mods</b> instalados neste servidor.</div>
      </div>
    </div>
  {:else}
    <div class="empty" style="min-height:200px;">
      <b>Catálogo</b> — pesquisa e instala plugins/mods aqui.
    </div>
  {/if}
</div>