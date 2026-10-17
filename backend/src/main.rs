use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::{Arc, Mutex}};
use sysinfo::System;
use tower_http::{cors::CorsLayer, services::ServeDir};

#[derive(Clone)]
struct AppState { inner: Arc<Mutex<PanelState>> }

#[derive(Serialize, Clone)]
struct LogLine { time: String, text: String, kind: String }

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
    logs: Vec<LogLine>,
    last_started: Option<DateTime<Utc>>,
}

#[derive(Deserialize)] struct CommandRequest { command: String }
#[derive(Deserialize)] struct ConfigRequest { server_name: String, version: String, server_dir: String, memory_limit_gb: u64 }

fn initial_state() -> PanelState {
    let java_path = std::env::var("JAVA_HOME").map(|p| format!("{p}/bin/java")).unwrap_or_else(|_| "java".into());
    PanelState {
        status: "offline".into(), address: "localhost:25565".into(), players: 0,
        memory_mb: 0, memory_limit_gb: 4, cpu_percent: 0.0, uptime_seconds: 0,
        server_name: "Sobrevivência principal".into(), version: "1.21.1".into(),
        server_dir: "~/Minecraft/MeuServidor".into(), java_detected: true, java_path,
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
        .route("/api/config", post(save_config)).route("/api/command", post(command))
        .with_state(app).layer(CorsLayer::permissive());
    let app = api.fallback_service(ServeDir::new("../frontend/dist").append_index_html_on_directories(true));
    let addr = SocketAddr::from(([0, 0, 0, 0], 4173));
    println!("ShadowPainel disponível em http://127.0.0.1:4173");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
