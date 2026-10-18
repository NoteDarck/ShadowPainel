<script lang="ts">
  import { onMount } from 'svelte';
  import { serverStore, serverActions } from '../stores/servers';

  const api = async <T=unknown>(path: string, options?: RequestInit): Promise<T> => {
    const res = await fetch(`/api${path}`, options);
    const text = await res.text();
    let data: any = {};
    try { data = text ? JSON.parse(text) : {}; } catch { throw new Error(`Erro ${res.status}`); }
    if (!res.ok) throw new Error(data.error || `Erro ${res.status}`);
    return data as T;
  };
  const json = (value: unknown): RequestInit => ({
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value)
  });

  let props: any = {
    motd: '', max_players: 20, gamemode: 'survival', difficulty: 'easy',
    white_list: false, online_mode: true, allow_flight: false, force_gamemode: false,
    spawn_protection: 0, require_resource_pack: false, resource_pack: '', resource_pack_prompt: '',
    timezone: 'America/Sao_Paulo', java_version: 'Java 21'
  };
  let loadingProps = false;
  let savedProps = false;

  onMount(async () => {
    try {
      const data = await api<any>('/server/properties');
      if (data) props = { ...props, ...data };
    } catch (e) {
      console.error('Falha ao carregar server.properties', e);
    }
  });

  async function saveServerProperties() {
    try {
      loadingProps = true;
      await api('/server/properties', json(props));
      savedProps = true;
      setTimeout(() => savedProps = false, 2200);
    } catch (e) {
      alert('Falha ao salvar server.properties.');
    } finally {
      loadingProps = false;
    }
  }
</script>

<section class="settings-card">
  <div class="card-head">
    <div><h3>Configurações avançadas</h3><p>Ajustes do servidor ativo.</p></div>
  </div>
  <div class="form-grid">
    <label>Nome do servidor<input bind:value={$serverStore.server_name}/></label>
    <label>Versão atual<input bind:value={$serverStore.version}/></label>
    <label>Pasta do servidor<input bind:value={$serverStore.server_dir}/></label>
    <label>Memória (GB)<input type="number" min="1" max="64" bind:value={$serverStore.memory_limit_gb}/></label>
  </div>
  <button class="primary" on:click={() => serverActions.saveConfig({
    server_name: $serverStore.server_name,
    version: $serverStore.version,
    server_dir: $serverStore.server_dir,
    memory_limit_gb: $serverStore.memory_limit_gb
  })}>SALVAR CONFIGURAÇÃO <span>→</span></button>
</section>

<section class="settings-card">
  <div class="card-head" style="padding-bottom: 16px;">
    <div><h3 style="display:flex; align-items:center; gap:8px;"><span style="font-size:20px; color:var(--accent);">📄</span> server.properties</h3></div>
    <div style="display:flex; gap:10px; align-items:center;">
      {#if savedProps}<span style="color:var(--green); font-size:12px;">✓ Salvo</span>{/if}
      <button class="secondary" on:click={saveServerProperties} disabled={loadingProps}>
        {loadingProps ? 'Salvando...' : 'Salvar'}
      </button>
    </div>
  </div>

  <div class="props-grid">
    <div class="prop-group" style="grid-column: 1 / 2;">
      <label>MOTD</label>
      <div class="prop-control"><input bind:value={props.motd} /></div>
    </div>
    <div class="prop-group" style="grid-column: 2 / 3;">
      <label>Vagas</label>
      <div class="prop-control"><input type="number" bind:value={props.max_players} /></div>
    </div>
    <div class="prop-group">
      <label>Modo de jogo</label>
      <div class="prop-control">
        <select bind:value={props.gamemode}>
          <option value="survival">Sobrevivência</option>
          <option value="creative">Criativo</option>
          <option value="adventure">Aventura</option>
          <option value="spectator">Espectador</option>
        </select>
      </div>
    </div>
    <div class="prop-group">
      <label>Dificuldade</label>
      <div class="prop-control">
        <select bind:value={props.difficulty}>
          <option value="peaceful">Pacífico</option>
          <option value="easy">Fácil</option>
          <option value="normal">Normal</option>
          <option value="hard">Difícil</option>
        </select>
      </div>
    </div>
    <div class="prop-group">
      <label>Whitelist</label>
      <div class="prop-control toggle-control">
        <button class="toggle-btn" class:active={props.white_list} on:click={() => props.white_list = !props.white_list}>
          {#if props.white_list}✓{:else}✕{/if}
        </button>
      </div>
    </div>
    <div class="prop-group">
      <label>Offline Mode</label>
      <div class="prop-control toggle-control">
        <button class="toggle-btn" class:active={!props.online_mode} on:click={() => props.online_mode = !props.online_mode}>
          {#if !props.online_mode}✓{:else}✕{/if}
        </button>
      </div>
    </div>
  </div>
</section>