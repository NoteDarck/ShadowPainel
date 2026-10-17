use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, Mutex},
};
use tower_http::services::ServeDir;

const MAX_LOGS: usize = 500;
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

type ApiErr = (StatusCode, Json<Value>);
type ApiResult = Result<Json<View>, ApiErr>;

fn err(code: StatusCode, msg: impl Into<String>) -> ApiErr {
    (code, Json(json!({ "error": msg.into() })))
}

// ───────────────────────── Configuração ─────────────────────────

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct E4allConfig {
    loader: String,
    minecraft_version: String,
    android_variant: bool,
    offline_accounts: bool,
    voice_chat: bool,
    install_path: String,
}

impl Default for E4allConfig {
    fn default() -> Self {
        Self {
            loader: "fabric".into(),
            minecraft_version: "1.21.1".into(),
            android_variant: false,
            offline_accounts: false,
            voice_chat: true,
            install_path: "~/Minecraft/MeuServidor/mods".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Config {
    server_name: String,
    version: String,
    server_dir: String,
    memory_limit_gb: u64,
    java_path: String,
    e4all: E4allConfig,
}

impl Default for Config {
    fn default() -> Self {
        let java_path = std::env::var("JAVA_HOME")
            .map(|p| format!("{p}/bin/java"))
            .unwrap_or_else(|_| "java".into());
        Self {
            server_name: "Sobrevivência principal".into(),
            version: "1.21.1".into(),
            server_dir: "~/Minecraft/MeuServidor".into(),
            memory_limit_gb: 4,
            java_path,
            e4all: E4allConfig::default(),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn expand(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(h) = home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(path)
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("SHADOWPAINEL_CONFIG") {
        return p.into();
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".shadowpainel")
        .join("config.json")
}

async fn load_config(path: &PathBuf) -> Config {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

async fn detect_java(path: &str) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

// ───────────────────────── Estado ─────────────────────────

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Status {
    Offline,
    Online,
    Stopping,
}

#[derive(Serialize, Clone)]
struct LogLine {
    time: String,
    text: String,
    kind: String,
}

struct Inner {
    cfg: Config,
    cfg_path: PathBuf,
    java_detected: bool,
    status: Status,
    logs: VecDeque<LogLine>,
    players: u32,
    started: Option<DateTime<Utc>>,
    pid: Option<u32>,
    stdin: Option<ChildStdin>,
    kill_tx: Option<oneshot::Sender<()>>,
    sys: System,
}

impl Inner {
    fn new(cfg: Config, cfg_path: PathBuf, java_detected: bool) -> Self {
        let mut s = Self {
            cfg,
            cfg_path,
            java_detected,
            status: Status::Offline,
            logs: VecDeque::new(),
            players: 0,
            started: None,
            pid: None,
            stdin: None,
            kill_tx: None,
            sys: System::new(),
        };
        s.push("ShadowPainel pronto. Aguardando inicialização.", "muted");
        s
    }

    fn push(&mut self, text: impl Into<String>, kind: &str) {
        if self.logs.len() >= MAX_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(LogLine {
            time: Local::now().format("%H:%M:%S").to_string(),
            text: text.into(),
            kind: kind.into(),
        });
    }

    async fn persist(&self) -> std::io::Result<()> {
        if let Some(parent) = self.cfg_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let data = serde_json::to_vec_pretty(&self.cfg).map_err(std::io::Error::other)?;
        tokio::fs::write(&self.cfg_path, data).await
    }
}

#[derive(Clone)]
struct AppState(Arc<Mutex<Inner>>);

#[derive(Serialize)]
struct View {
    status: Status,
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
}

fn view(i: &mut Inner) -> View {
    let (mut memory_mb, mut cpu_percent) = (0u64, 0.0f32);
    if let Some(pid) = i.pid {
        let p = Pid::from_u32(pid);
        i.sys.refresh_processes(ProcessesToUpdate::Some(&[p]), true);
        if let Some(proc_) = i.sys.process(p) {
            memory_mb = proc_.memory() / 1024 / 1024;
            let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
            cpu_percent = (proc_.cpu_usage() / cores).min(100.0);
        }
    }
    let uptime_seconds = i
        .started
        .map(|s| (Utc::now() - s).num_seconds().max(0) as u64)
        .unwrap_or(0);
    View {
        status: i.status,
        address: "localhost:25565".into(),
        players: i.players,
        memory_mb,
        memory_limit_gb: i.cfg.memory_limit_gb,
        cpu_percent,
        uptime_seconds,
        server_name: i.cfg.server_name.clone(),
        version: i.cfg.version.clone(),
        server_dir: i.cfg.server_dir.clone(),
        java_path: i.cfg.java_path.clone(),
        java_detected: i.java_detected,
        e4all: i.cfg.e4all.clone(),
        logs: i.logs.iter().cloned().collect(),
    }
}

// ───────────────────────── Processo do servidor ─────────────────────────

async fn pump<R: AsyncRead + Unpin>(app: AppState, reader: R) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut i = app.0.lock().await;
        if line.contains(" joined the game") {
            i.players += 1;
        } else if line.contains(" left the game") {
            i.players = i.players.saturating_sub(1);
        }
        let kind = if line.contains("ERROR") {
            "error"
        } else if line.contains("WARN") {
            "warning"
        } else if line.contains("Done (") {
            "success"
        } else {
            "info"
        };
        i.push(line, kind);
    }
}

async fn wait_child(app: AppState, mut child: Child, kill_rx: oneshot::Receiver<()>) {
    let result = tokio::select! {
        r = child.wait() => r,
        k = kill_rx => {
            if k.is_ok() {
                let _ = child.kill().await;
            }
            child.wait().await
        }
    };
    let mut i = app.0.lock().await;
    i.status = Status::Offline;
    i.pid = None;
    i.stdin = None;
    i.kill_tx = None;
    i.players = 0;
    i.started = None;
    match result {
        Ok(code) => i.push(format!("Servidor encerrado ({code})."), "warning"),
        Err(e) => i.push(format!("Erro ao aguardar o processo: {e}"), "error"),
    }
}

async fn send_line(i: &mut Inner, line: &str) -> bool {
    match i.stdin.as_mut() {
        Some(s) => {
            s.write_all(format!("{line}\n").as_bytes()).await.is_ok() && s.flush().await.is_ok()
        }
        None => false,
    }
}

// ───────────────────────── Handlers ─────────────────────────

async fn health() -> &'static str {
    "ok"
}

async fn state(State(app): State<AppState>) -> Json<View> {
    let mut i = app.0.lock().await;
    Json(view(&mut i))
}

async fn start(State(app): State<AppState>) -> ApiResult {
    let mut i = app.0.lock().await;
    if i.status != Status::Offline {
        return Err(err(StatusCode::CONFLICT, "O servidor já está em execução"));
    }
    let dir = expand(&i.cfg.server_dir);
    if !dir.join("server.jar").is_file() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("server.jar não encontrado em {}", dir.display()),
        ));
    }
    let mut cmd = Command::new(&i.cfg.java_path);
    cmd.current_dir(&dir)
        .arg(format!("-Xmx{}G", i.cfg.memory_limit_gb))
        .args(["-jar", "server.jar", "nogui"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao iniciar o Java ({}): {e}", i.cfg.java_path),
        )
    })?;

    i.pid = child.id();
    i.stdin = child.stdin.take();
    if let Some(o) = child.stdout.take() {
        tokio::spawn(pump(app.clone(), o));
    }
    if let Some(e) = child.stderr.take() {
        tokio::spawn(pump(app.clone(), e));
    }
    let (tx, rx) = oneshot::channel();
    i.kill_tx = Some(tx);
    i.status = Status::Online;
    i.started = Some(Utc::now());
    i.players = 0;
    let java = i.cfg.java_path.clone();
    i.push(format!("Iniciando servidor · Java: {java}"), "success");
    tokio::spawn(wait_child(app.clone(), child, rx));
    Ok(Json(view(&mut i)))
}

async fn stop(State(app): State<AppState>) -> ApiResult {
    let mut i = app.0.lock().await;
    if i.status != Status::Online {
        return Err(err(StatusCode::CONFLICT, "O servidor não está online"));
    }
    i.status = Status::Stopping;
    send_line(&mut i, "stop").await;
    i.push("Parando o servidor (comando stop)…", "warning");
    let a = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(STOP_TIMEOUT).await;
        let mut i = a.0.lock().await;
        if i.status == Status::Stopping {
            if let Some(tx) = i.kill_tx.take() {
                i.push("Tempo esgotado: encerrando o processo à força.", "error");
                let _ = tx.send(());
            }
        }
    });
    Ok(Json(view(&mut i)))
}

#[derive(Deserialize)]
struct ConfigRequest {
    server_name: String,
    version: String,
    server_dir: String,
    memory_limit_gb: u64,
}

async fn save_config(State(app): State<AppState>, Json(req): Json<ConfigRequest>) -> ApiResult {
    if req.server_name.trim().is_empty()
        || req.version.trim().is_empty()
        || req.server_dir.trim().is_empty()
    {
        return Err(err(StatusCode::BAD_REQUEST, "Nome, versão e pasta são obrigatórios"));
    }
    if !(1..=64).contains(&req.memory_limit_gb) {
        return Err(err(StatusCode::BAD_REQUEST, "Memória deve ficar entre 1 e 64 GB"));
    }
    let mut i = app.0.lock().await;
    i.cfg.server_name = req.server_name.trim().into();
    i.cfg.version = req.version.trim().into();
    i.cfg.server_dir = req.server_dir.trim().into();
    i.cfg.memory_limit_gb = req.memory_limit_gb;
    if let Err(e) = i.persist().await {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("Falha ao gravar a configuração: {e}")));
    }
    let msg = if i.status == Status::Offline {
        "Configuração salva."
    } else {
        "Configuração salva. Será aplicada na próxima inicialização."
    };
    i.push(msg, "success");
    Ok(Json(view(&mut i)))
}

#[derive(Deserialize)]
struct E4allRequest {
    loader: String,
    minecraft_version: String,
    android_variant: bool,
    offline_accounts: bool,
    voice_chat: bool,
    install_path: String,
}

async fn save_e4all(State(app): State<AppState>, Json(req): Json<E4allRequest>) -> ApiResult {
    const LOADERS: [&str; 4] = ["fabric", "forge", "neoforge", "quilt"];
    if !LOADERS.contains(&req.loader.as_str())
        || req.minecraft_version.trim().is_empty()
        || req.install_path.trim().is_empty()
    {
        return Err(err(StatusCode::BAD_REQUEST, "Loader, versão e pasta do e4all são obrigatórios"));
    }
    let mut i = app.0.lock().await;
    i.cfg.e4all = E4allConfig {
        loader: req.loader,
        minecraft_version: req.minecraft_version.trim().into(),
        android_variant: req.android_variant,
        offline_accounts: req.offline_accounts,
        voice_chat: req.voice_chat,
        install_path: req.install_path.trim().into(),
    };
    if let Err(e) = i.persist().await {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("Falha ao gravar a configuração: {e}")));
    }
    i.push("Configuração do e4all salva. Instale o JAR correspondente na pasta mods.", "success");
    Ok(Json(view(&mut i)))
}

#[derive(Deserialize)]
struct CommandRequest {
    command: String,
}

async fn command(
    State(app): State<AppState>,
    Json(req): Json<CommandRequest>,
) -> Result<Json<Value>, ApiErr> {
    let cmd = req.command.trim();
    if cmd.is_empty() || cmd.contains(['\n', '\r']) {
        return Err(err(StatusCode::BAD_REQUEST, "Comando inválido"));
    }
    let mut i = app.0.lock().await;
    if i.status != Status::Online {
        return Err(err(StatusCode::CONFLICT, "O servidor precisa estar online para receber comandos"));
    }
    if !send_line(&mut i, cmd).await {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Falha ao enviar o comando ao servidor"));
    }
    i.push(format!("> {cmd}"), "command");
    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── Infra ─────────────────────────

/// Protege contra DNS rebinding quando o painel está preso ao loopback.
async fn host_guard(req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let host = if h.starts_with('[') {
                h.split(']').next().map(|s| format!("{s}]")).unwrap_or_default()
            } else {
                h.split(':').next().unwrap_or("").to_string()
            };
            matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
        })
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "Host inválido").into_response()
    }
}

fn frontend_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SHADOWPAINEL_FRONTEND_DIR") {
        return p.into();
    }
    let mut c: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            c.push(d.join("../frontend/dist"));
            c.push(d.join("../../frontend/dist"));
            c.push(d.join("../../../frontend/dist"));
        }
    }
    c.extend(
        [
            "../frontend/dist",
            "frontend/dist",
            "/usr/share/shadowpainel/frontend",
            "/usr/local/share/shadowpainel/frontend",
        ]
        .map(PathBuf::from),
    );
    c.into_iter()
        .find(|p| p.join("index.html").is_file())
        .unwrap_or_else(|| PathBuf::from("../frontend/dist"))
}

async fn shutdown_server(app: &AppState) {
    {
        let mut i = app.0.lock().await;
        if i.status != Status::Online {
            return;
        }
        i.status = Status::Stopping;
        send_line(&mut i, "stop").await;
    }
    println!("Encerrando o servidor Minecraft…");
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if app.0.lock().await.status == Status::Offline {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg_path = config_path();
    let cfg = load_config(&cfg_path).await;
    let java_detected = detect_java(&cfg.java_path).await;
    let app_state = AppState(Arc::new(Mutex::new(Inner::new(cfg, cfg_path, java_detected))));

    let addr: SocketAddr = std::env::var("SHADOWPAINEL_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:4173".into())
        .parse()?;

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(state))
        .route("/api/server/start", post(start))
        .route("/api/server/stop", post(stop))
        .route("/api/config", post(save_config))
        .route("/api/e4all/config", post(save_e4all))
        .route("/api/command", post(command))
        .with_state(app_state.clone());

    let mut app =
        api.fallback_service(ServeDir::new(frontend_dir()).append_index_html_on_directories(true));
    if addr.ip().is_loopback() {
        app = app.layer(middleware::from_fn(host_guard));
    } else {
        eprintln!("AVISO: o painel não tem autenticação e está exposto em {addr}.");
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("ShadowPainel disponível em http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    shutdown_server(&app_state).await;
    Ok(())
}
