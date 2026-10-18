<script lang="ts">
  import { onMount } from 'svelte';
  import { serverStore, serverActions, noticeStore, savedStore } from './stores/servers';
  import ServerSwitcher from './ServerSwitcher.svelte';
  import Dashboard from './pages/Dashboard.svelte';
  import Settings from './pages/Settings.svelte';
  import Content from './pages/Content.svelte';
  // import Console from './pages/Console.svelte'; // Se for usar

  let currentPage = 'dashboard';
  const APP_VERSION = '0.7.3';

  onMount(() => {
    // Inicia o polling
    serverActions.refresh();
    const timer = setInterval(() => serverActions.refresh(), 3000);

    // Roteamento simples
    const handleHashChange = () => {
      const hash = window.location.hash.replace('#/', '') || 'dashboard';
      currentPage = hash;
    };
    window.addEventListener('hashchange', handleHashChange);
    handleHashChange();

    return () => {
      clearInterval(timer);
      window.removeEventListener('hashchange', handleHashChange);
    };
  });
</script>

<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="mark">S</div>
      <div><strong>Shadow<span>Painel</span></strong><small>SERVER CONTROL</small></div>
    </div>

    <ServerSwitcher />

    <nav>
      <div class="nav-label">SISTEMA</div>
      <button class:active={currentPage === 'dashboard'} on:click={() => window.location.hash = '#/dashboard'}>
        <span class="nav-icon">◈</span>Visão geral
      </button>
      <button class:active={currentPage === 'content'} on:click={() => window.location.hash = '#/content'}>
        <span class="nav-icon">▦</span>Conteúdo
      </button>
      <button class:active={currentPage === 'settings'} on:click={() => window.location.hash = '#/settings'}>
        <span class="nav-icon">⚙</span>Configurações
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
        <div class="breadcrumb">SERVIDORES <span>/</span> {$serverStore.server_name}</div>
        <h1>
          {currentPage === 'content' ? 'Conteúdo do servidor'
           : currentPage === 'settings' ? 'Configurações'
           : 'Visão geral'} <span class="wave">✦</span>
        </h1>
      </div>
      <div class="header-actions">
        <button class="icon-button">◔</button>
        <button class="help">? <span>Ajuda</span></button>
      </div>
    </header>

    {#if $noticeStore}<div class="notice">{$noticeStore}</div>{/if}
    {#if $savedStore}<div class="notice success-notice">Configuração salva.</div>{/if}

    {#if currentPage === 'dashboard'}
      <Dashboard />
    {:else if currentPage === 'settings'}
      <Settings />
    {:else if currentPage === 'content'}
      <!-- Importe e coloque o Content.svelte aqui -->
      <div class="empty">Página de conteúdo em construção.</div>
    {/if}

    <footer>
      <span>SHADOWPAINEL <b>v{APP_VERSION}</b></span>
      <span>RUST CORE · MULTIPLATAFORMA <i>●</i></span>
    </footer>
  </main>
</div>