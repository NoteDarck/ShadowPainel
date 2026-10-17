use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::{get, post}, Json, Router};
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
    logs: Vec<LogLine>,
    last_started: Option<DateTime<Utc>>,
}

#[derive(Deserialize)] struct CommandRequest { command: String }

fn initial_state() -> PanelState {
    PanelState {
        status: "offline".into(), address: "localhost:25565".into(), players: 0,
        memory_mb: 0, memory_limit_gb: 4, cpu_percent: 0.0, uptime_seconds: 0,
        server_name: "Sobrevivência principal".into(), version: "1.21.1".into(),
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

async fn start(State(app): State<AppState>) -> Json<PanelState> {
    let mut s = app.inner.lock().unwrap();
    s.status = "online".into(); s.last_started = Some(Utc::now()); s.players = 0; s.memory_mb = 742;
    s.logs.push(LogLine { time: "agora".into(), text: "Servidor iniciado com Java 21 · porta 25565".into(), kind: "success".into() });
    Json(s.clone())
}
async fn stop(State(app): State<AppState>) -> Json<PanelState> {
    let mut s = app.inner.lock().unwrap(); s.status = "offline".into(); s.players = 0; s.memory_mb = 0; s.cpu_percent = 0.0;
    s.logs.push(LogLine { time: "agora".into(), text: "Servidor encerrado com segurança.".into(), kind: "warning".into() }); Json(s.clone())
}
async fn command(State(app): State<AppState>, Json(req): Json<CommandRequest>) -> impl IntoResponse {
    if req.command.trim().is_empty() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"Comando vazio"}))); }
    let mut s = app.inner.lock().unwrap();
    s.logs.push(LogLine { time: "agora".into(), text: format!("> {}", req.command), kind: "command".into() });
    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
}

#[tokio::main]
async fn main() {
    let app = AppState { inner: Arc::new(Mutex::new(initial_state())) };
    let api = Router::new().route("/api/health", get(health)).route("/api/state", get(state)).route("/api/server/start", post(start)).route("/api/server/stop", post(stop)).route("/api/command", post(command)).with_state(app).layer(CorsLayer::permissive());
    let app = api.fallback_service(ServeDir::new("../frontend/dist").append_index_html_on_directories(true));
    let addr = SocketAddr::from(([0, 0, 0, 0], 4173));
    println!("ShadowPainel disponível em http://127.0.0.1:4173");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
