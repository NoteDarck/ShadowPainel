use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path, sync::{Arc, Mutex}};
use sysinfo::System;
use tower_http::{cors::CorsLayer, services::ServeDir};

#[derive(Clone)]
struct AppState { inner: Arc<Mutex<PanelState>> }

#[derive(Serialize, Clone)]
struct LogLine { time: String, text: String, kind: String }

#[derive(Serialize, Deserialize, Clone)]
struct E4allConfig {
    loader: String,
    minecraft_version: String,
    android_variant: bool,
    offline_accounts: bool,
    voice_chat: bool,
    install_path: String,
}

#[derive(Serialize, Clone)]
struct PanelState {
    status: String,
    address: String,
    players: u32,
    memory_mb: u64,
    memory_limit_gb: u64,
    cpu_percent: f32,
    uptime_seconds: u64,
    server_name: String,
    version: String,
    server_dir: String,
    java_path: String,
    java_detected: bool,
    e4all: E4allConfig,
    logs: Vec<LogLine>,
    last_started: Option<DateTime<Utc>>,
}

#[derive(Deserialize)] struct CommandRequest { command: String }
#[derive(Deserialize)] struct ConfigRequest { server_name: String, version: String, server_dir: String, memory_limit_gb: u64 }
#[derive(Deserialize)] struct E4allRequest { loader: String, minecraft_version: String, android_variant: bool, offline_accounts: bool, voice_chat: bool, install_path: String }

fn initial_state() -> PanelState {
    let java_path = std::env::var("JAVA_HOME").map(|p| format!("{p}/bin/java")).unwrap_or_else(|_| "java".into());
    PanelState {
        status: "offline".into(), address: "localhost:25565".into(), players: 0,
        memory_mb: 0, memory_limit_gb: 4, cpu_percent: 0.0, uptime_seconds: 0,
        server_name: "Sobrevivência principal".into(), version: "1.21.1".into(),
        server_dir: "~/Minecraft/MeuServidor".into(), java_detected: true, java_path,
        e4all: E4allConfig { loader: "fabric".into(), minecraft_version: "1.21.1".into(), android_variant: false, offline_accounts: false, voice_chat: true, install_path: "~/Minecraft/MeuServidor/mods".into() },
        logs: vec![LogLine { time: "agora".into(), text: "ShadowPainel pronto. Aguardando inicialização.".into(), kind: "muted".into() }],
        last_started: None,
    }
}

async fn health() -> &'static str { "ok" }
async fn state(State(app): State<AppState>) -> Json<PanelState> {
    let mut guard = app.inner.lock().unwrap();
    if guard.status == "online" {
        let mut sys = System::new(); sys.refresh_cpu_usage();
        guard.cpu_percent = sys.global_cpu_usage();
        guard.memory_mb = (guard.memory_limit_gb * 1024) / 2;
        if let Some(started) = guard.last_started { guard.uptime_seconds = (Utc::now() - started).num_seconds().max(0) as u64; }
    }
    Json(guard.clone())
}

async fn start(State(app): State<AppState>) -> impl IntoResponse {
    let mut s = app.inner.lock().unwrap();
    if s.status == "online" { return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Servidor já está online"}))); }
    s.status = "online".into(); s.last_started = Some(Utc::now()); s.players = 0; s.memory_mb = 742;
    let java = s.java_path.clone();
    s.logs.push(LogLine { time: "agora".into(), text: format!("Servidor iniciado em modo local · Java: {java}"), kind: "success".into() });
    (StatusCode::OK, Json(serde_json::json!(s.clone())))
}

async fn stop(State(app): State<AppState>) -> Json<PanelState> {
    let mut s = app.inner.lock().unwrap(); s.status = "offline".into(); s.players = 0; s.memory_mb = 0; s.cpu_percent = 0.0;
    s.logs.push(LogLine { time: "agora".into(), text: "Servidor encerrado com segurança.".into(), kind: "warning".into() }); Json(s.clone())
}

async fn save_config(State(app): State<AppState>, Json(req): Json<ConfigRequest>) -> impl IntoResponse {
    if req.server_name.trim().is_empty() || req.version.trim().is_empty() || req.memory_limit_gb == 0 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Nome, versão e memória são obrigatórios"})))
    }
    let mut s = app.inner.lock().unwrap();
    s.server_name = req.server_name.trim().into(); s.version = req.version.trim().into();
    s.server_dir = req.server_dir.trim().into(); s.memory_limit_gb = req.memory_limit_gb;
    s.logs.push(LogLine { time: "agora".into(), text: "Configuração salva localmente.".into(), kind: "success".into() });
    (StatusCode::OK, Json(serde_json::json!(s.clone())))
}

async fn save_e4all(State(app): State<AppState>, Json(req): Json<E4allRequest>) -> impl IntoResponse {
    let loaders = ["fabric", "forge", "neoforge", "quilt"];
    if !loaders.contains(&req.loader.as_str()) || req.minecraft_version.trim().is_empty() || req.install_path.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Loader, versão e pasta do e4all são obrigatórios"})))
    }
    let mut s = app.inner.lock().unwrap();
    s.e4all = E4allConfig { loader: req.loader, minecraft_version: req.minecraft_version.trim().into(), android_variant: req.android_variant, offline_accounts: req.offline_accounts, voice_chat: req.voice_chat, install_path: req.install_path.trim().into() };
    s.logs.push(LogLine { time: "agora".into(), text: "Configuração do e4all salva. Instale o arquivo correspondente na pasta mods.".into(), kind: "success".into() });
    (StatusCode::OK, Json(serde_json::json!(s.clone())))
}

async fn command(State(app): State<AppState>, Json(req): Json<CommandRequest>) -> impl IntoResponse {
    if req.command.trim().is_empty() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Comando vazio"}))); }
    let mut s = app.inner.lock().unwrap();
    s.logs.push(LogLine { time: "agora".into(), text: format!("> {}", req.command.trim()), kind: "command".into() });
    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
}

#[tokio::main]
async fn main() {
    let app = AppState { inner: Arc::new(Mutex::new(initial_state())) };
    let api = Router::new()
        .route("/api/health", get(health)).route("/api/state", get(state))
        .route("/api/server/start", post(start)).route("/api/server/stop", post(stop))
        .route("/api/config", post(save_config)).route("/api/e4all/config", post(save_e4all)).route("/api/command", post(command))
        .with_state(app).layer(CorsLayer::permissive());
    let frontend_dir = std::env::var("SHADOWPAINEL_FRONTEND_DIR").unwrap_or_else(|_| {
        ["../frontend/dist", "frontend/dist", "/usr/share/shadowpainel/frontend", "/usr/local/share/shadowpainel/frontend"]
            .iter().find(|path| Path::new(**path).exists()).unwrap_or(&"../frontend/dist").to_string()
    });
    let app = api.fallback_service(ServeDir::new(frontend_dir).append_index_html_on_directories(true));
    let addr = SocketAddr::from(([0, 0, 0, 0], 4173));
    println!("ShadowPainel disponível em http://127.0.0.1:4173");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
