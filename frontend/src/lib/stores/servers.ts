import { writable } from 'svelte/store';

export type ServerInstance = {
  id: string;
  server_name: string;
  version: string;
  server_dir: string;
  memory_limit_gb: number;
};

export type ViewState = {
  status: 'offline' | 'online' | 'stopping';
  address: string;
  players: number;
  memory_mb: number;
  memory_limit_gb: number;
  cpu_percent: number;
  uptime_seconds: number;
  server_name: string;
  version: string;
  server_dir: string;
  java_path: string;
  java_detected: boolean;
  installed: boolean;
  e4all: any;
  logs: { time: string; text: string; kind: string }[];
  servers: ServerInstance[];
  active_server_id: string | null;
};

// Estado inicial padrão
const defaultState: ViewState = {
  status: 'offline', address: 'localhost:25565', players: 0, memory_mb: 0,
  memory_limit_gb: 4, cpu_percent: 0, uptime_seconds: 0,
  server_name: 'Carregando...', version: '',
  server_dir: '', java_path: 'java', java_detected: false, installed: false,
  e4all: { loader: 'fabric', minecraft_version: '', android_variant: false, offline_accounts: false, voice_chat: true, install_path: '', public_address: '' },
  logs: [], servers: [], active_server_id: null
};

export const serverStore = writable<ViewState>(defaultState);
export const loadingStore = writable(false);
export const noticeStore = writable('');
export const savedStore = writable(false);

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

export const serverActions = {
  async refresh() {
    try {
      const data = await api<ViewState>('/state');
      serverStore.set(data);
    } catch {
      noticeStore.set('Núcleo Rust offline ou ainda iniciando.');
    }
  },

  async switchServer(id: string) {
    loadingStore.set(true);
    try {
      const data = await api<ViewState>('/server/switch', json({ id }));
      serverStore.set(data);
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Falha ao trocar de servidor.');
    }
    loadingStore.set(false);
  },

  async createServer(payload: { server_name: string; version: string; loader: string; server_dir: string; memory_limit_gb: number; }) {
    loadingStore.set(true);
    try {
      const data = await api<ViewState>('/server/create', json(payload));
      serverStore.set(data);
      noticeStore.set('Servidor instalado. Aceite a EULA antes de iniciar.');
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Falha na instalação.');
    }
    loadingStore.set(false);
  },

  async toggleServer() {
    loadingStore.set(true);
    let current: ViewState;
    serverStore.subscribe(s => current = s)();
    try {
      const data = await api<ViewState>(
        current!.status === 'online' ? '/server/stop' : '/server/start',
        { method: 'POST' }
      );
      serverStore.set(data);
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Falha ao controlar servidor.');
    }
    loadingStore.set(false);
  },

  async acceptEula() {
    try {
      await api('/server/eula', { method: 'POST' });
      noticeStore.set('EULA aceita. Você já pode iniciar o servidor.');
      await this.refresh();
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Falha ao aceitar EULA.');
    }
  },

  async saveConfig(payload: { server_name: string; version: string; server_dir: string; memory_limit_gb: number; }) {
    try {
      const data = await api<ViewState>('/config', json(payload));
      serverStore.set(data);
      savedStore.set(true);
      setTimeout(() => savedStore.set(false), 2200);
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Falha ao salvar.');
    }
  },

  async sendCommand(command: string) {
    if (!command.trim()) return;
    try {
      await api('/command', json({ command }));
      await this.refresh();
    } catch (e) {
      noticeStore.set(e instanceof Error ? e.message : 'Console indisponível.');
    }
  }
};