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
    collections::VecDeque, net::SocketAddr, path::PathBuf, process::Stdio, sync::Arc,
    time::Duration,
};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, Mutex},
};
use tower_http::services::ServeDir;

const APP_VERSION: &str = "0.7.0";

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
    public_address: String,
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
            public_address: "".into(),
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

fn server_port(dir: &std::path::Path) -> u16 {
    std::fs::read_to_string(dir.join("server.properties"))
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "server-port")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
        })
        .unwrap_or(25565)
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
        let temp = self.cfg_path.with_extension("json.tmp");
        tokio::fs::write(&temp, data).await?;
        tokio::fs::rename(temp, &self.cfg_path).await
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
    installed: bool,
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
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1) as f32;
            cpu_percent = (proc_.cpu_usage() / cores).min(100.0);
        }
    }
    let dir = expand(&i.cfg.server_dir);
    let installed = dir.join("server.jar").is_file()
        || dir.join("fabric-server-launch.jar").is_file();
    let uptime_seconds = i
        .started
        .map(|s| (Utc::now() - s).num_seconds().max(0) as u64)
        .unwrap_or(0);
    let address = format!("localhost:{}", server_port(&dir));
    View {
        status: i.status,
        address,
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
        installed,
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

#[derive(Deserialize)]
struct DownloadRequest {
    version: String,
}

async fn download_server(
    State(app): State<AppState>,
    Json(req): Json<DownloadRequest>,
) -> ApiResult {
    let mut i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar pasta: {e}"),
        )
    })?;
    let jar_path = dir.join("server.jar");

    if jar_path.exists() {
        return Err(err(
            StatusCode::CONFLICT,
            "O server.jar já existe nesta pasta",
        ));
    }

    let manifest_url = "https://launchermeta.mojang.com/mc/game/version_manifest.json";
    let client = reqwest::Client::new();
    let manifest: Value = client
        .get(manifest_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha na rede: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Serviço externo retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("JSON inválido: {e}"),
            )
        })?;

    let version_url = manifest["versions"]
        .as_array()
        .and_then(|versions| {
            versions.iter().find_map(|v| {
                if v["id"] == req.version {
                    v["url"].as_str().map(String::from)
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                format!("Versão {} não encontrada", req.version),
            )
        })?;

    let version_data: Value = client
        .get(&version_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha na rede: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Serviço externo retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("JSON inválido: {e}"),
            )
        })?;

    let server_url = version_data["downloads"]["server"]["url"]
        .as_str()
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "server.jar não disponível para esta versão",
            )
        })?;

    let bytes = client
        .get(server_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha no download: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Download rejeitado: {e}")))?
        .bytes()
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha na leitura: {e}"),
            )
        })?;

    tokio::fs::write(&jar_path, &bytes).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar: {e}"),
        )
    })?;

    i.cfg.version = req.version.clone();
    let _ = i.persist().await;
    i.push(
        format!("server.jar da versão {} baixado.", req.version),
        "success",
    );
    Ok(Json(view(&mut i)))
}

async fn accept_eula(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    let eula_path = dir.join("eula.txt");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar pasta: {e}"),
        )
    })?;

    let content = format!(
        "# EULA aceita via ShadowPainel em {}\neula=true\n",
        Local::now()
    );
    tokio::fs::write(&eula_path, content).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao escrever eula.txt: {e}"),
        )
    })?;

    Ok(Json(json!({ "ok": true, "message": "EULA aceita." })))
}

async fn start(State(app): State<AppState>) -> ApiResult {
    let mut i = app.0.lock().await;
    if i.status != Status::Offline {
        return Err(err(StatusCode::CONFLICT, "O servidor já está em execução"));
    }
    let dir = expand(&i.cfg.server_dir);
    if !dir.join("server.jar").is_file() && !dir.join("fabric-server-launch.jar").is_file() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "server.jar não encontrado. Use a tela de download.",
        ));
    }
    let eula_path = dir.join("eula.txt");
    let eula_accepted = tokio::fs::read_to_string(&eula_path)
        .await
        .ok()
        .is_some_and(|contents| {
            contents
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("eula=true"))
        });
    if !eula_accepted {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Aceite a EULA antes de iniciar.",
        ));
    }

    let mut cmd = Command::new(&i.cfg.java_path);
    let jar_name = if dir.join("fabric-server-launch.jar").is_file() {
        "fabric-server-launch.jar"
    } else {
        "server.jar"
    };

    cmd.current_dir(&dir)
        .arg(format!("-Xmx{}G", i.cfg.memory_limit_gb))
        .args(["-jar", jar_name, "nogui"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao iniciar o Java: {e}"),
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
    i.push(format!("Iniciando servidor com {}", jar_name), "success");
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
                i.push("Tempo esgotado: encerrando à força.", "error");
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

fn validate_config(req: &ConfigRequest) -> Result<(), String> {
    if req.server_name.trim().is_empty()
        || req.version.trim().is_empty()
        || req.server_dir.trim().is_empty()
    {
        return Err("Nome, versão e pasta são obrigatórios".into());
    }
    if !(1..=64).contains(&req.memory_limit_gb) {
        return Err("Memória deve ficar entre 1 e 64 GB".into());
    }
    Ok(())
}

async fn save_config(State(app): State<AppState>, Json(req): Json<ConfigRequest>) -> ApiResult {
    validate_config(&req).map_err(|message| err(StatusCode::BAD_REQUEST, message))?;
    let mut i = app.0.lock().await;
    i.cfg.server_name = req.server_name.trim().into();
    i.cfg.version = req.version.trim().into();
    i.cfg.server_dir = req.server_dir.trim().into();
    i.cfg.memory_limit_gb = req.memory_limit_gb;
    if let Err(e) = i.persist().await {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao gravar: {e}"),
        ));
    }
    i.push("Configuração salva.", "success");
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
    public_address: String,
}

fn validate_e4all(req: &E4allRequest) -> Result<(), String> {
    const LOADERS: [&str; 4] = ["fabric", "forge", "neoforge", "quilt"];
    if !LOADERS.contains(&req.loader.as_str())
        || req.minecraft_version.trim().is_empty()
        || req.install_path.trim().is_empty()
    {
        return Err("Loader, versão e pasta do e4all são obrigatórios".into());
    }
    Ok(())
}

async fn save_e4all(State(app): State<AppState>, Json(req): Json<E4allRequest>) -> ApiResult {
    validate_e4all(&req).map_err(|message| err(StatusCode::BAD_REQUEST, message))?;
    let mut i = app.0.lock().await;
    i.cfg.e4all = E4allConfig {
        loader: req.loader,
        minecraft_version: req.minecraft_version.trim().into(),
        android_variant: req.android_variant,
        offline_accounts: req.offline_accounts,
        voice_chat: req.voice_chat,
        install_path: req.install_path.trim().into(),
        public_address: req.public_address.trim().into(),
    };
    if let Err(e) = i.persist().await {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao gravar: {e}"),
        ));
    }
    i.push("Configuração do e4all salva.", "success");
    Ok(Json(view(&mut i)))
}
#[derive(Deserialize)]
struct DeleteServerRequest {
    confirm_name: String,
}

async fn delete_server(
    State(app): State<AppState>,
    Json(req): Json<DeleteServerRequest>,
) -> ApiResult {
    let mut i = app.0.lock().await;

    if i.status != Status::Offline {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor antes de excluí-lo.",
        ));
    }

    let expected = i.cfg.server_name.trim();
    if req.confirm_name.trim() != expected {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "O nome digitado não corresponde ao nome do servidor.",
        ));
    }

    let dir = expand(&i.cfg.server_dir);
    if dir.exists() {
        // Segurança: só remove se a pasta realmente contém um servidor
        let looks_like_server = dir.join("server.jar").is_file()
            || dir.join("fabric-server-launch.jar").is_file()
            || dir.join("server.properties").is_file()
            || dir.join("eula.txt").is_file();
        if !looks_like_server {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "A pasta configurada não parece ser de um servidor. Exclusão cancelada por segurança.",
            ));
        }
        tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao remover a pasta do servidor: {e}"),
            )
        })?;
    }

    // Reseta estado
    i.cfg = Config::default();
    i.status = Status::Offline;
    i.players = 0;
    i.started = None;
    i.pid = None;
    i.stdin = None;
    i.kill_tx = None;
    i.logs.clear();
    i.push(
        "Servidor excluído. Crie uma nova instalação para continuar.",
        "warning",
    );

    if let Err(e) = i.persist().await {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao gravar config: {e}"),
        ));
    }

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
    if cmd.is_empty() || cmd.contains(['\n', '\r']) || cmd.len() > 4096 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Comando inválido ou muito longo",
        ));
    }
    let mut i = app.0.lock().await;
    if i.status != Status::Online {
        return Err(err(StatusCode::CONFLICT, "Servidor offline."));
    }
    if !send_line(&mut i, cmd).await {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Falha ao enviar comando.",
        ));
    }
    i.push(format!("> {cmd}"), "command");
    Ok(Json(json!({ "ok": true })))
}

// --- FUNÇÕES PARA MODS E FABRIC ---

#[derive(Deserialize)]
struct ModSearchRequest {
    query: String,
    loader: String,
    version: String,
}

async fn search_mods(Json(req): Json<ModSearchRequest>) -> Result<Json<Value>, ApiErr> {
    let client = reqwest::Client::new();
    let facets = format!(
        r#"[["categories:{}"],["versions:{}"]]"#,
        req.loader, req.version
    );
    let url = format!(
        "https://api.modrinth.com/v2/search?query={}&facets={}&limit=10",
        urlencoding::encode(&req.query),
        urlencoding::encode(&facets)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Rede: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Serviço externo retornou erro: {e}"),
            )
        })?;
    let data: Value = resp
        .json()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON: {e}")))?;
    Ok(Json(data))
}

#[derive(Deserialize)]
struct ModInstallRequest {
    project_id: String,
    loader: String,
    version: String,
}

async fn install_mod(
    State(app): State<AppState>,
    Json(req): Json<ModInstallRequest>,
) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    let mods_dir = dir.join("mods");
    tokio::fs::create_dir_all(&mods_dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Pasta mods: {e}"),
        )
    })?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.modrinth.com/v2/project/{}/version?loaders=[\"{}\"]&game_versions=[\"{}\"]",
        req.project_id, req.loader, req.version
    );
    let resp = client
        .get(&url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Rede: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Serviço externo retornou erro: {e}"),
            )
        })?;
    let versions: Vec<Value> = resp
        .json()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON: {e}")))?;

    if versions.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "Mod incompatível."));
    }

    let file = &versions[0]["files"][0];
    let download_url = file["url"]
        .as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "URL faltando"))?;
    let filename = file["filename"]
        .as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Nome faltando"))?;

    let bytes = client
        .get(download_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Download: {e}")))?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Download rejeitado: {e}")))?
        .bytes()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Leitura: {e}")))?;

    let safe_name = std::path::Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name == filename && !name.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Nome de arquivo do mod inválido"))?;
    let path = mods_dir.join(safe_name);
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Salvar: {e}")))?;

    Ok(Json(json!({ "ok": true, "file": filename })))
}

async fn install_fabric(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Pasta: {e}")))?;

    let client = reqwest::Client::new();
    let resp = client
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Rede: {e}")))?;
    let installers: Vec<Value> = resp
        .json()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON: {e}")))?;

    if installers.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "Instalador não encontrado."));
    }
    let download_url = installers[0]["url"]
        .as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "URL faltando"))?;

    let bytes = client
        .get(download_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Download: {e}")))?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Download rejeitado: {e}")))?
        .bytes()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Leitura: {e}")))?;

    let path = dir.join("fabric-server-launch.jar");
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Salvar: {e}")))?;

    Ok(Json(json!({ "ok": true })))
}

// ───────────────────────── Instalação e conteúdo ─────────────────────────

#[derive(Serialize)]
struct MinecraftVersion {
    id: String,
    kind: String,
    release_time: String,
}

async fn minecraft_versions() -> Result<Json<Value>, ApiErr> {
    let client = reqwest::Client::new();
    let manifest: Value = client
        .get("https://launchermeta.mojang.com/mc/game/version_manifest.json")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao consultar versões: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Mojang retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Manifesto de versões inválido: {e}"),
            )
        })?;
    let versions: Vec<MinecraftVersion> = manifest["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some(MinecraftVersion {
                id: v["id"].as_str()?.to_string(),
                kind: v["type"].as_str().unwrap_or("release").to_string(),
                release_time: v["releaseTime"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    Ok(Json(
        json!({"latest": manifest["latest"], "versions": versions}),
    ))
}

#[derive(Deserialize)]
struct CreateServerRequest {
    server_name: String,
    version: String,
    loader: String,
    server_dir: String,
    memory_limit_gb: u64,
}

fn valid_loader(loader: &str) -> bool {
    matches!(loader, "vanilla" | "fabric" | "paper" | "purpur")
}

async fn download_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, ApiErr> {
    client
        .get(url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Falha no download: {e}")))?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Download rejeitado: {e}")))?
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao ler download: {e}"),
            )
        })
}

async fn create_server(
    State(app): State<AppState>,
    Json(req): Json<CreateServerRequest>,
) -> ApiResult {
    if req.server_name.trim().is_empty()
        || req.version.trim().is_empty()
        || req.server_dir.trim().is_empty()
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Nome, versão e pasta são obrigatórios.",
        ));
    }
    if !valid_loader(&req.loader) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Loader não suportado nesta versão. Use Vanilla, Fabric, Paper ou Purpur.",
        ));
    }
    if !(1..=64).contains(&req.memory_limit_gb) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Memória deve ficar entre 1 e 64 GB.",
        ));
    }
    let dir = expand(&req.server_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar pasta: {e}"),
        )
    })?;
    let client = reqwest::Client::new();
    let jar_path = dir.join("server.jar");
    if jar_path.exists() || dir.join("fabric-server-launch.jar").exists() {
        return Err(err(
            StatusCode::CONFLICT,
            "Esta pasta já possui uma instalação de servidor.",
        ));
    }

    match req.loader.as_str() {
        "vanilla" => {
            let manifest: Value = client
                .get("https://launchermeta.mojang.com/mc/game/version_manifest.json")
                .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
                .send()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Falha ao consultar Mojang: {e}"),
                    )
                })?
                .error_for_status()
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Mojang retornou erro: {e}"),
                    )
                })?
                .json()
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Manifesto inválido: {e}")))?;
            let metadata_url = manifest["versions"]
                .as_array()
                .and_then(|list| list.iter().find(|v| v["id"] == req.version))
                .and_then(|v| v["url"].as_str())
                .ok_or_else(|| err(StatusCode::NOT_FOUND, "Versão Minecraft não encontrada."))?;
            let metadata: Value = client
                .get(metadata_url)
                .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
                .send()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Falha ao consultar versão: {e}"),
                    )
                })?
                .error_for_status()
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Mojang retornou erro: {e}"),
                    )
                })?
                .json()
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Metadados inválidos: {e}")))?;
            let url = metadata["downloads"]["server"]["url"]
                .as_str()
                .ok_or_else(|| {
                    err(
                        StatusCode::NOT_FOUND,
                        "Esta versão não oferece server.jar oficial.",
                    )
                })?;
            tokio::fs::write(&jar_path, download_bytes(&client, url).await?)
                .await
                .map_err(|e| {
                    err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Falha ao salvar server.jar: {e}"),
                    )
                })?;
        }
        "fabric" => {
            let loaders: Vec<Value> = client
                .get(format!(
                    "https://meta.fabricmc.net/v2/versions/loader/{}",
                    req.version
                ))
                .send()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Falha ao consultar Fabric: {e}"),
                    )
                })?
                .error_for_status()
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Fabric retornou erro: {e}"),
                    )
                })?
                .json()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Resposta Fabric inválida: {e}"),
                    )
                })?;
            let loader = loaders
                .first()
                .and_then(|v| v["loader"]["version"].as_str())
                .ok_or_else(|| {
                    err(
                        StatusCode::NOT_FOUND,
                        "Fabric não possui loader para esta versão.",
                    )
                })?;
            let installer: Vec<Value> = client
                .get("https://meta.fabricmc.net/v2/versions/installer")
                .send()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Falha ao consultar instalador Fabric: {e}"),
                    )
                })?
                .error_for_status()
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Fabric retornou erro: {e}"),
                    )
                })?
                .json()
                .await
                .map_err(|e| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        format!("Instalador Fabric inválido: {e}"),
                    )
                })?;
            let installer_version = installer
                .first()
                .and_then(|v| v["version"].as_str())
                .ok_or_else(|| err(StatusCode::NOT_FOUND, "Instalador Fabric não encontrado."))?;
            let url = format!("https://meta.fabricmc.net/v2/versions/loader/{}/{loader}/{installer_version}/server/jar", req.version);
            tokio::fs::write(
                &dir.join("fabric-server-launch.jar"),
                download_bytes(&client, &url).await?,
            )
            .await
            .map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Falha ao salvar servidor Fabric: {e}"),
                )
            })?;
        }
        "paper" | "purpur" => {
            let url = if req.loader == "paper" {
                let builds: Value = client
                    .get(format!(
                        "https://api.papermc.io/v2/projects/paper/versions/{}",
                        req.version
                    ))
                    .send()
                    .await
                    .map_err(|e| {
                        err(
                            StatusCode::BAD_GATEWAY,
                            format!("Falha ao consultar Paper: {e}"),
                        )
                    })?
                    .error_for_status()
                    .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Paper retornou erro: {e}")))?
                    .json()
                    .await
                    .map_err(|e| {
                        err(
                            StatusCode::BAD_GATEWAY,
                            format!("Resposta Paper inválida: {e}"),
                        )
                    })?;
                let build = builds["builds"]
                    .as_array()
                    .and_then(|b| b.last())
                    .and_then(|b| b.as_i64())
                    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Build Paper não encontrada."))?;
                format!("https://api.papermc.io/v2/projects/paper/versions/{}/builds/{}/downloads/paper-{}-{}.jar", req.version, build, req.version, build)
            } else {
                let builds: Value = client
                    .get(format!(
                        "https://api.purpurmc.org/v2/purpur/{}",
                        req.version
                    ))
                    .send()
                    .await
                    .map_err(|e| {
                        err(
                            StatusCode::BAD_GATEWAY,
                            format!("Falha ao consultar Purpur: {e}"),
                        )
                    })?
                    .error_for_status()
                    .map_err(|e| {
                        err(
                            StatusCode::BAD_GATEWAY,
                            format!("Purpur retornou erro: {e}"),
                        )
                    })?
                    .json()
                    .await
                    .map_err(|e| {
                        err(
                            StatusCode::BAD_GATEWAY,
                            format!("Resposta Purpur inválida: {e}"),
                        )
                    })?;
                let build = builds["builds"]
                    .as_array()
                    .and_then(|b| b.last())
                    .and_then(|b| b.as_str())
                    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Build Purpur não encontrada."))?;
                format!(
                    "https://api.purpurmc.org/v2/purpur/{}/{}/download",
                    req.version, build
                )
            };
            tokio::fs::write(&jar_path, download_bytes(&client, &url).await?)
                .await
                .map_err(|e| {
                    err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Falha ao salvar servidor: {e}"),
                    )
                })?;
        }
        _ => unreachable!(),
    }
    let mut i = app.0.lock().await;
    i.cfg.server_name = req.server_name.trim().into();
    i.cfg.version = req.version.trim().into();
    i.cfg.server_dir = req.server_dir.trim().into();
    i.cfg.memory_limit_gb = req.memory_limit_gb;
    i.persist().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar configuração: {e}"),
        )
    })?;
    i.push(
        format!("Servidor {} {} instalado.", req.loader, req.version),
        "success",
    );
    Ok(Json(view(&mut i)))
}

#[derive(Deserialize)]
struct ContentQuery {
    kind: Option<String>,
}

fn content_dir(base: &std::path::Path, kind: &str) -> Option<PathBuf> {
    match kind {
        "mods" | "plugins" | "datapacks" => Some(base.join(kind)),
        _ => None,
    }
}

#[derive(Serialize)]
struct ContentItem {
    filename: String,
    size: u64,
    kind: String,
    project_id: Option<String>,
    version_number: Option<String>,
    icon_url: Option<String>,
    installed_version: Option<String>,
}

async fn list_content(
    State(app): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ContentQuery>,
) -> Result<Json<Value>, ApiErr> {
    let kind = query.kind.unwrap_or_else(|| "mods".into());
    let i = app.0.lock().await;
    let base = expand(&i.cfg.server_dir);
    let dir = content_dir(&base, &kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Tipo de conteúdo inválido."))?;
    drop(i);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha na pasta: {e}"),
        )
    })?;
    let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao listar conteúdo: {e}"),
        )
    })?;
    let mut items = Vec::new();
    
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao ler conteúdo: {e}"),
        )
    })? {
        let metadata = entry.metadata().await.ok();
        if metadata.as_ref().is_some_and(|m| m.is_file()) {
            let filename = entry.file_name().to_string_lossy().into_owned();
            let version_number = Some(filename.trim_end_matches(".jar").to_string());
            
            items.push(ContentItem {
                filename,
                size: metadata.unwrap().len(),
                kind: kind.clone(),
                project_id: None,
                version_number: version_number.clone(),
                icon_url: None,
                installed_version: version_number,
            });
        }
    }
    items.sort_by_key(|a| a.filename.to_lowercase());
    Ok(Json(json!({"items": items})))
}

#[derive(Deserialize)]
struct ContentDeleteRequest {
    kind: String,
    filenames: Vec<String>,
}

async fn delete_content(
    State(app): State<AppState>,
    Json(req): Json<ContentDeleteRequest>,
) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let dir = content_dir(&expand(&i.cfg.server_dir), &req.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Tipo de conteúdo inválido."))?;
    drop(i);

    let mut deleted = Vec::new();
    let mut errors = Vec::new();

    for filename in req.filenames {
        let safe_name = std::path::Path::new(&filename)
            .file_name()
            .and_then(|x| x.to_str())
            .filter(|x| *x == filename && !x.is_empty());
        
        if let Some(name) = safe_name {
            let path = dir.join(name);
            if path.exists() {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    errors.push(format!("{name}: {e}"));
                } else {
                    deleted.push(name.to_string());
                }
            } else {
                errors.push(format!("{name}: não encontrado"));
            }
        } else {
            errors.push(format!("{filename}: nome inválido"));
        }
    }

    if !errors.is_empty() {
        return Ok(Json(json!({"ok": false, "deleted": deleted, "errors": errors})));
    }

    Ok(Json(json!({"ok": true, "deleted": deleted})))
}

#[derive(Deserialize)]
struct ContentSearchRequest {
    query: String,
    kind: String,
    loader: String,
    version: String,
    category: Option<String>,
}

async fn search_content(Json(req): Json<ContentSearchRequest>) -> Result<Json<Value>, ApiErr> {
    let project_type = match req.kind.as_str() {
        "plugins" => "plugin",
        "datapacks" => "datapack",
        _ => "mod",
    };
    let loader_facet = if req.kind == "mods" && req.loader != "vanilla" {
        format!(r#",["categories:{}"]"#, req.loader)
    } else {
        String::new()
    };
    
    let category_facet = match req.category {
        Some(cat) if !cat.is_empty() && cat != "all" => format!(r#",["categories:{}"]"#, cat),
        _ => String::new(),
    };

    let facets = format!(
        r#"[["project_type:{}"],["versions:{}"]{}{}]"#,
        project_type, req.version, loader_facet, category_facet
    );
    
    let url = format!(
        "https://api.modrinth.com/v2/search?query={}&facets={}&limit=12",
        urlencoding::encode(&req.query),
        urlencoding::encode(&facets)
    );
    
    let data: Value = reqwest::Client::new()
        .get(url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Falha na busca: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Modrinth retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Resposta inválida: {e}")))?;
    Ok(Json(data))
}

// NOVO: Rota para buscar todas as versões de um mod específico
async fn get_content_versions(
    axum::extract::Query(query): axum::extract::Query<ContentSearchRequest>,
) -> Result<Json<Value>, ApiErr> {
    let url = format!(
        "https://api.modrinth.com/v2/project/{}/version?loaders=[\"{}\"]&game_versions=[\"{}\"]",
        urlencoding::encode(&query.query),
        urlencoding::encode(&query.loader),
        urlencoding::encode(&query.version)
    );

    let data: Value = reqwest::Client::new()
        .get(url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Falha ao buscar versões: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Modrinth retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Resposta inválida: {e}")))?;

    Ok(Json(data))
}

#[derive(Deserialize)]
struct ContentInstallRequest {
    project_id: String,
    kind: String,
    loader: String,
    version: String,
    version_id: Option<String>, // NOVO: versão específica do mod escolhida
}

async fn install_content(
    State(app): State<AppState>,
    Json(req): Json<ContentInstallRequest>,
) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let base = expand(&i.cfg.server_dir);
    let dir = content_dir(&base, &req.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Tipo de conteúdo inválido."))?;
    drop(i);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha na pasta: {e}"),
        )
    })?;
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.modrinth.com/v2/project/{}/version?loaders=[\"{}\"]&game_versions=[\"{}\"]",
        urlencoding::encode(&req.project_id),
        urlencoding::encode(&req.loader),
        urlencoding::encode(&req.version)
    );
    let versions: Vec<Value> = client
        .get(url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao consultar projeto: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Modrinth retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Resposta inválida: {e}")))?;
    
    // NOVO: Se version_id foi passado, filtra a versão exata
    let version = if let Some(vid) = &req.version_id {
        versions.iter().find(|v| v["id"].as_str() == Some(vid))
    } else {
        versions.first()
    };

    let file = version
        .and_then(|v| v["files"].as_array())
        .and_then(|files| {
            files
                .iter()
                .find(|f| f["primary"].as_bool().unwrap_or(false))
                .or_else(|| files.first())
        })
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "Nenhum arquivo compatível encontrado.",
            )
        })?;
    let filename = file["filename"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Nome de arquivo ausente."))?;
    let safe = std::path::Path::new(filename)
        .file_name()
        .and_then(|x| x.to_str())
        .filter(|x| *x == filename)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Nome de arquivo inválido."))?;
    let download = file["url"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "URL de download ausente."))?;
    tokio::fs::write(dir.join(safe), download_bytes(&client, download).await?)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao salvar conteúdo: {e}"),
            )
        })?;
    Ok(Json(json!({"ok": true, "filename": safe})))
}

// ───────────────────────── Infra ─────────────────────────

async fn host_guard(req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let host = if h.starts_with('[') {
                h.split(']')
                    .next()
                    .map(|s| format!("{s}]"))
                    .unwrap_or_default()
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
    let app_state = AppState(Arc::new(Mutex::new(Inner::new(
        cfg,
        cfg_path,
        java_detected,
    ))));

    let configured_addr = std::env::var("SHADOWPAINEL_ADDR").ok();
    let requested: SocketAddr = configured_addr
        .as_deref()
        .unwrap_or("127.0.0.1:4173")
        .parse()?;
    let listener = if configured_addr.is_some() {
        tokio::net::TcpListener::bind(requested).await?
    } else {
        let mut selected = None;
        for port in requested.port()..=requested.port().saturating_add(20) {
            if let Ok(l) =
                tokio::net::TcpListener::bind(SocketAddr::new(requested.ip(), port)).await
            {
                selected = Some(l);
                break;
            }
        }
        selected.ok_or_else(|| std::io::Error::other("nenhuma porta disponível"))?
    };
    let addr = listener.local_addr()?;

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(state))
        .route("/api/server/start", post(start))
        .route("/api/server/stop", post(stop))
        .route("/api/server/delete", post(delete_server))
        .route("/api/server/download", post(download_server))
        .route("/api/server/eula", post(accept_eula))
        .route("/api/server/install-fabric", post(install_fabric))
        .route("/api/minecraft/versions", get(minecraft_versions))
        .route("/api/server/create", post(create_server))
        .route("/api/content", get(list_content))
        .route("/api/content/delete", post(delete_content))
        .route("/api/content/search", post(search_content))
        .route("/api/content/versions", get(get_content_versions)) // NOVO
        .route("/api/content/install", post(install_content))
        .route("/api/mods/search", post(search_mods))
        .route("/api/mods/install", post(install_mod))
        .route("/api/config", post(save_config))
        .route("/api/e4all/config", post(save_e4all))
        .route("/api/command", post(command))
        .with_state(app_state.clone());

    let mut app =
        api.fallback_service(ServeDir::new(frontend_dir()).append_index_html_on_directories(true));
    if addr.ip().is_loopback() {
        app = app.layer(middleware::from_fn(host_guard));
    }

    println!("ShadowPainel disponível em http://{addr}");
    use std::io::Write;
    std::io::stdout().flush()?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    shutdown_server(&app_state).await;
    Ok(())
}