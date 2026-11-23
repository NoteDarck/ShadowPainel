import AsyncStorage from '@react-native-async-storage/async-storage';
import { StatusBar } from 'expo-status-bar';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  RefreshControl,
  SafeAreaView,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';

type ServerStatus = 'offline' | 'starting' | 'online' | 'stopping' | string;
type ServerState = {
  status: ServerStatus;
  server_name: string;
  version: string;
  loader: string;
  players: number;
  memory_mb: number;
  memory_limit_gb: number;
  cpu_percent: number;
  uptime_seconds: number;
  installed: boolean;
  eula_accepted: boolean;
  logs: { time: string; text: string; kind: string }[];
};
type ServerSummary = {
  server_name: string;
  version: string;
  server_dir: string;
  installed: boolean;
  active: boolean;
  status: ServerStatus;
};

const SAVED_URL = 'shadowpainel.android.url';
const DEFAULT_URL = 'http://192.168.1.100:4173';

function normalizeUrl(value: string) {
  const trimmed = value.trim().replace(/\/+$/, '');
  return trimmed || DEFAULT_URL;
}

function statusLabel(status: ServerStatus) {
  if (status === 'online') return 'ONLINE';
  if (status === 'starting') return 'INICIANDO';
  if (status === 'stopping') return 'PARANDO';
  return 'OFFLINE';
}

function formatUptime(seconds: number) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return `${hours}h ${minutes}min`;
}

export default function App() {
  const [baseUrl, setBaseUrl] = useState(DEFAULT_URL);
  const [draftUrl, setDraftUrl] = useState(DEFAULT_URL);
  const [connected, setConnected] = useState(false);
  const [state, setState] = useState<ServerState | null>(null);
  const [servers, setServers] = useState<ServerSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [message, setMessage] = useState('Informe o endereço do ShadowPainel no computador.');

  const endpoint = useMemo(() => normalizeUrl(baseUrl), [baseUrl]);

  const api = useCallback(async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetch(`${endpoint}/api${path}`, {
      ...init,
      headers: { Accept: 'application/json', ...(init?.headers || {}) },
    });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    return payload as T;
  }, [endpoint]);

  const refresh = useCallback(async (silent = false) => {
    if (!silent) setRefreshing(true);
    try {
      const [nextState, nextServers] = await Promise.all([
        api<ServerState>('/state'),
        api<ServerSummary[]>('/servers'),
      ]);
      setState(nextState);
      setServers(nextServers);
      setConnected(true);
      setMessage('Conectado ao ShadowPainel.');
    } catch (error) {
      setConnected(false);
      setMessage(error instanceof Error ? error.message : 'Não foi possível conectar.');
    } finally {
      setRefreshing(false);
    }
  }, [api]);

  useEffect(() => {
    AsyncStorage.getItem(SAVED_URL).then((saved) => {
      if (saved) {
        setBaseUrl(saved);
        setDraftUrl(saved);
      }
    });
  }, []);

  useEffect(() => {
    if (connected) {
      const timer = setInterval(() => refresh(true), 5000);
      return () => clearInterval(timer);
    }
  }, [connected, refresh]);

  async function connect() {
    const nextUrl = normalizeUrl(draftUrl);
    setBaseUrl(nextUrl);
    await AsyncStorage.setItem(SAVED_URL, nextUrl);
    setLoading(true);
    await refresh();
    setLoading(false);
  }

  async function action(path: '/server/start' | '/server/stop') {
    setLoading(true);
    try {
      await api(path, { method: 'POST' });
      await refresh(true);
    } catch (error) {
      Alert.alert('ShadowPainel', error instanceof Error ? error.message : 'Ação não concluída.');
    } finally {
      setLoading(false);
    }
  }

  async function switchServer(server: ServerSummary) {
    if (server.active) return;
    setLoading(true);
    try {
      await api('/servers/switch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ server_dir: server.server_dir }),
      });
      await refresh(true);
    } catch (error) {
      Alert.alert('ShadowPainel', error instanceof Error ? error.message : 'Não foi possível trocar de servidor.');
    } finally {
      setLoading(false);
    }
  }

  const online = state?.status === 'online';

  return (
    <SafeAreaView style={styles.safe}>
      <StatusBar style="light" />
      <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView
          contentContainerStyle={styles.container}
          refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => refresh()} tintColor="#b9a7ff" />}
        >
          <View style={styles.header}>
            <View>
              <Text style={styles.eyebrow}>MINECRAFT CONTROL</Text>
              <Text style={styles.title}>ShadowPainel</Text>
              <Text style={styles.subtitle}>Controle seu servidor pelo Android.</Text>
            </View>
            <View style={[styles.connectionDot, connected && styles.connectionDotOn]} />
          </View>

          <View style={styles.card}>
            <Text style={styles.cardTitle}>Conectar ao computador</Text>
            <Text style={styles.help}>Use o IP local do PC e a porta do painel. Exemplo: http://192.168.1.25:4173</Text>
            <TextInput
              value={draftUrl}
              onChangeText={setDraftUrl}
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              placeholder="http://192.168.1.25:4173"
              placeholderTextColor="#71809a"
              style={styles.input}
            />
            <Pressable style={styles.primaryButton} onPress={connect} disabled={loading}>
              {loading ? <ActivityIndicator color="#fff" /> : <Text style={styles.primaryText}>{connected ? 'ATUALIZAR CONEXÃO' : 'CONECTAR'}</Text>}
            </Pressable>
            <Text style={[styles.message, connected ? styles.success : styles.warning]}>{message}</Text>
          </View>

          {connected && state && (
            <>
              <View style={styles.heroCard}>
                <View style={styles.heroTop}>
                  <View>
                    <Text style={styles.eyebrow}>SERVIDOR ATIVO</Text>
                    <Text style={styles.serverName}>{state.server_name}</Text>
                    <Text style={styles.meta}>{state.version || 'sem versão'} · {state.loader}</Text>
                  </View>
                  <View style={[styles.statusPill, online ? styles.statusOnline : styles.statusOffline]}>
                    <Text style={styles.statusText}>{statusLabel(state.status)}</Text>
                  </View>
                </View>
                <View style={styles.metrics}>
                  <Metric label="JOGADORES" value={String(state.players)} />
                  <Metric label="RAM" value={`${state.memory_mb} MB`} />
                  <Metric label="CPU" value={`${state.cpu_percent.toFixed(1)}%`} />
                  <Metric label="UPTIME" value={formatUptime(state.uptime_seconds)} />
                </View>
                <View style={styles.actions}>
                  <Pressable style={[styles.primaryButton, online && styles.dangerButton]} onPress={() => action(online ? '/server/stop' : '/server/start')} disabled={loading || state.status === 'starting' || state.status === 'stopping'}>
                    <Text style={styles.primaryText}>{online ? 'PARAR SERVIDOR' : 'INICIAR SERVIDOR'}</Text>
                  </Pressable>
                  <Pressable style={styles.secondaryButton} onPress={() => refresh()} disabled={loading}>
                    <Text style={styles.secondaryText}>ATUALIZAR</Text>
                  </Pressable>
                </View>
              </View>

              <View style={styles.card}>
                <Text style={styles.cardTitle}>Meus servidores</Text>
                {servers.map((server) => (
                  <Pressable key={server.server_dir} style={[styles.serverRow, server.active && styles.serverRowActive]} onPress={() => switchServer(server)}>
                    <View style={styles.serverRowText}>
                      <Text style={styles.rowTitle}>{server.server_name}</Text>
                      <Text style={styles.rowMeta}>{server.version || 'sem versão'} · {server.active ? 'ativo' : 'toque para selecionar'}</Text>
                    </View>
                    <Text style={[styles.rowStatus, server.status === 'online' && styles.success]}>{statusLabel(server.status)}</Text>
                  </Pressable>
                ))}
                {servers.length === 0 && <Text style={styles.help}>Nenhum perfil adicional encontrado.</Text>}
              </View>

              <View style={styles.card}>
                <Text style={styles.cardTitle}>Console recente</Text>
                {(state.logs || []).slice(-8).map((log, index) => (
                  <Text key={`${log.time}-${index}`} style={styles.logLine} numberOfLines={2}>{log.text}</Text>
                ))}
                {!state.logs?.length && <Text style={styles.help}>Nenhum log disponível.</Text>}
              </View>
            </>
          )}
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return <View style={styles.metric}><Text style={styles.metricLabel}>{label}</Text><Text style={styles.metricValue}>{value}</Text></View>;
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#0b0d14' },
  flex: { flex: 1 },
  container: { padding: 20, gap: 16 },
  header: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'flex-start', paddingTop: 10, paddingBottom: 6 },
  eyebrow: { color: '#a995ff', fontSize: 11, fontWeight: '800', letterSpacing: 1.5 },
  title: { color: '#f4f2ff', fontSize: 32, fontWeight: '800', marginTop: 4 },
  subtitle: { color: '#a8b1c4', fontSize: 14, marginTop: 4 },
  connectionDot: { width: 14, height: 14, borderRadius: 7, backgroundColor: '#ef6f84', marginTop: 8 },
  connectionDotOn: { backgroundColor: '#64d89a' },
  card: { backgroundColor: '#121722', borderColor: '#293247', borderWidth: 1, borderRadius: 16, padding: 18, gap: 12 },
  heroCard: { backgroundColor: '#17152a', borderColor: 'rgba(143,107,255,.5)', borderWidth: 1, borderRadius: 18, padding: 18, gap: 18 },
  cardTitle: { color: '#eef1f8', fontSize: 18, fontWeight: '800' },
  help: { color: '#a8b1c4', fontSize: 13, lineHeight: 19 },
  input: { backgroundColor: '#0b0d14', color: '#f4f2ff', borderColor: '#3b4660', borderWidth: 1, borderRadius: 10, paddingHorizontal: 14, minHeight: 48, fontSize: 15 },
  primaryButton: { backgroundColor: '#8f6bff', borderRadius: 10, minHeight: 48, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 16 },
  dangerButton: { backgroundColor: '#b94f68' },
  primaryText: { color: '#fff', fontSize: 13, fontWeight: '800', letterSpacing: .5 },
  secondaryButton: { borderColor: '#4c5770', borderWidth: 1, borderRadius: 10, minHeight: 48, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 16 },
  secondaryText: { color: '#d8d2ff', fontSize: 13, fontWeight: '800' },
  message: { fontSize: 12 },
  success: { color: '#64d89a' },
  warning: { color: '#efb866' },
  heroTop: { flexDirection: 'row', justifyContent: 'space-between', gap: 12 },
  serverName: { color: '#fff', fontSize: 24, fontWeight: '800', marginTop: 5 },
  meta: { color: '#a8b1c4', fontSize: 13, marginTop: 4 },
  statusPill: { alignSelf: 'flex-start', borderRadius: 20, paddingHorizontal: 10, paddingVertical: 7 },
  statusOnline: { backgroundColor: 'rgba(100,216,154,.18)' },
  statusOffline: { backgroundColor: 'rgba(239,184,102,.16)' },
  statusText: { color: '#fff', fontSize: 10, fontWeight: '800', letterSpacing: 1 },
  metrics: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  metric: { backgroundColor: 'rgba(255,255,255,.06)', borderRadius: 10, padding: 11, minWidth: '46%', flexGrow: 1 },
  metricLabel: { color: '#71809a', fontSize: 9, fontWeight: '800', letterSpacing: 1 },
  metricValue: { color: '#f4f2ff', fontSize: 17, fontWeight: '800', marginTop: 4 },
  actions: { flexDirection: 'row', gap: 10 },
  serverRow: { flexDirection: 'row', alignItems: 'center', gap: 12, borderBottomColor: '#293247', borderBottomWidth: 1, paddingVertical: 13 },
  serverRowActive: { borderLeftColor: '#8f6bff', borderLeftWidth: 3, paddingLeft: 10 },
  serverRowText: { flex: 1 },
  rowTitle: { color: '#f4f2ff', fontSize: 15, fontWeight: '700' },
  rowMeta: { color: '#8d99b0', fontSize: 12, marginTop: 3 },
  rowStatus: { color: '#efb866', fontSize: 10, fontWeight: '800' },
  logLine: { color: '#b9c2d4', fontSize: 11, lineHeight: 18, fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace' },
});
