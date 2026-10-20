use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
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

const APP_VERSION: &str = "0.7.8";

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
    minecraft_dir: String,
    use_broker: bool,
    broker_url: String,
    relay_host: String,
    relay_port: u32,
    restore_dedicated_commands: bool,
    use_whitelist: bool,
    host_enabled: bool,
    prevent_pause_while_hosting: bool,
    dialtone_host_enabled: bool,
    dialtone_player_enabled: bool,
    dialtone_relay_map: String,
    dialtone_sanitize_ticket: bool,
    reconnect_max_attempts: u32,
    reconnect_base_delay_seconds: u32,
    keepalive_interval_seconds: u32,
    login_ready_timeout_ms: u32,
    relay_connect_timeout_ms: u32,
    offline_mode: bool,
    hide_domain_in_chat: bool,
    voice_p2p_enabled: bool,
}

fn default_e4all_fields() -> E4allConfig {
    E4allConfig {
        minecraft_dir: default_minecraft_dir(),
        use_broker: true,
        broker_url: "https://broker.e4mc.link/getBestRelay".into(),
        relay_host: "test.e4mc.link".into(),
        relay_port: 25575,
        restore_dedicated_commands: true,
        use_whitelist: false,
        host_enabled: true,
        prevent_pause_while_hosting: true,
        dialtone_host_enabled: true,
        dialtone_player_enabled: true,
        dialtone_relay_map: "https://natives.e4mc.link/relaymap.json".into(),
        dialtone_sanitize_ticket: true,
        reconnect_max_attempts: 5,
        reconnect_base_delay_seconds: 2,
        keepalive_interval_seconds: 15,
        login_ready_timeout_ms: 15000,
        relay_connect_timeout_ms: 15000,
        offline_mode: false,
        hide_domain_in_chat: false,
        voice_p2p_enabled: true,
    }
}

impl Default for E4allConfig {
    fn default() -> Self {
        default_e4all_fields()
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

// ───────────────────────── e4all.toml ─────────────────────────

fn default_minecraft_dir() -> String {
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return format!("{appdata}\\.minecraft");
        }
    }
    if cfg!(target_os = "macos") {
        return "~/Library/Application Support/minecraft".into();
    }
    "~/.minecraft".into()
}

fn config_file(dir: &str) -> PathBuf {
    expand(dir.trim()).join("config").join("e4all.toml")
}

fn is_key(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .map(|rest| rest.trim_start().starts_with('='))
        .unwrap_or(false)
}

fn read_raw(content: &str, key: &str) -> Option<String> {
    let line = content.lines().find(|l| is_key(l, key))?;
    let value = line.split_once('=')?.1.trim();
    if let Some(rest) = value.strip_prefix('"') {
        return rest.split_once('"').map(|(s, _)| s.to_string());
    }
    Some(value.split('#').next()?.trim().to_string())
}

fn set_value(content: &str, key: &str, rendered: &str) -> String {
    let mut found = false;
    let mut lines: Vec<String> = content
        .lines()
        .map(|l| {
            if is_key(l, key) {
                found = true;
                format!("{key} = {rendered}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("{key} = {rendered}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn apply_toml(c: &mut E4allConfig, t: &str) {
    fn b(t: &str, k: &str, x: &mut bool) {
        if let Some(v) = read_raw(t, k).and_then(|v| v.parse::<bool>().ok()) {
            *x = v;
        }
    }
    fn n(t: &str, k: &str, x: &mut u32) {
        if let Some(v) = read_raw(t, k).and_then(|v| v.parse::<u32>().ok()) {
            *x = v;
        }
    }
    fn s(t: &str, k: &str, x: &mut String) {
        if let Some(v) = read_raw(t, k) {
            *x = v;
        }
    }
    b(t, "useBroker", &mut c.use_broker);
    s(t, "brokerUrl", &mut c.broker_url);
    s(t, "relayHost", &mut c.relay_host);
    n(t, "relayPort", &mut c.relay_port);
    b(
        t,
        "restoreDedicatedCommands",
        &mut c.restore_dedicated_commands,
    );
    b(t, "useWhiteList", &mut c.use_whitelist);
    b(t, "hostEnabled", &mut c.host_enabled);
    b(
        t,
        "preventPauseWhileHosting",
        &mut c.prevent_pause_while_hosting,
    );
    b(t, "dialtoneHostEnabled", &mut c.dialtone_host_enabled);
    b(t, "dialtonePlayerEnabled", &mut c.dialtone_player_enabled);
    s(t, "dialtoneRelayMap", &mut c.dialtone_relay_map);
    b(t, "dialtoneSanitizeTicket", &mut c.dialtone_sanitize_ticket);
    n(t, "reconnectMaxAttempts", &mut c.reconnect_max_attempts);
    n(
        t,
        "reconnectBaseDelaySeconds",
        &mut c.reconnect_base_delay_seconds,
    );
    n(
        t,
        "keepaliveIntervalSeconds",
        &mut c.keepalive_interval_seconds,
    );
    n(t, "loginReadyTimeoutMs", &mut c.login_ready_timeout_ms);
    n(t, "relayConnectTimeoutMs", &mut c.relay_connect_timeout_ms);
    b(t, "offlineMode", &mut c.offline_mode);
    b(t, "hideDomainInChat", &mut c.hide_domain_in_chat);
    b(t, "voiceP2PEnabled", &mut c.voice_p2p_enabled);
}

fn toml_pairs(c: &E4allConfig) -> Vec<(&'static str, String)> {
    let q = |s: &str| format!("\"{s}\"");
    vec![
        ("useBroker", c.use_broker.to_string()),
        ("brokerUrl", q(&c.broker_url)),
        ("relayHost", q(&c.relay_host)),
        ("relayPort", c.relay_port.to_string()),
        (
            "restoreDedicatedCommands",
            c.restore_dedicated_commands.to_string(),
        ),
        ("useWhiteList", c.use_whitelist.to_string()),
        ("hostEnabled", c.host_enabled.to_string()),
        (
            "preventPauseWhileHosting",
            c.prevent_pause_while_hosting.to_string(),
        ),
        ("dialtoneHostEnabled", c.dialtone_host_enabled.to_string()),
        (
            "dialtonePlayerEnabled",
            c.dialtone_player_enabled.to_string(),
        ),
        ("dialtoneRelayMap", q(&c.dialtone_relay_map)),
        (
            "dialtoneSanitizeTicket",
            c.dialtone_sanitize_ticket.to_string(),
        ),
        ("reconnectMaxAttempts", c.reconnect_max_attempts.to_string()),
        (
            "reconnectBaseDelaySeconds",
            c.reconnect_base_delay_seconds.to_string(),
        ),
        (
            "keepaliveIntervalSeconds",
            c.keepalive_interval_seconds.to_string(),
        ),
        ("loginReadyTimeoutMs", c.login_ready_timeout_ms.to_string()),
        (
            "relayConnectTimeoutMs",
            c.relay_connect_timeout_ms.to_string(),
        ),
        ("offlineMode", c.offline_mode.to_string()),
        ("hideDomainInChat", c.hide_domain_in_chat.to_string()),
        ("voiceP2PEnabled", c.voice_p2p_enabled.to_string()),
    ]
}

fn load_e4all(dir: &str) -> E4allConfig {
    let mut cfg = default_e4all_fields();
    cfg.minecraft_dir = dir.trim().to_string();
    if let Ok(content) = fs::read_to_string(config_file(dir)) {
        apply_toml(&mut cfg, &content);
    }
    cfg
}

fn write_e4all_toml(cfg: &E4allConfig) -> Result<PathBuf, String> {
    let path = config_file(&cfg.minecraft_dir);
    let mut content = fs::read_to_string(&path).unwrap_or_default();
    for (key, value) in toml_pairs(cfg) {
        content = set_value(&content, key, &value);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Não foi possível criar a pasta config: {e}"))?;
    }
    fs::write(&path, content)
        .map_err(|e| format!("Não foi possível gravar {}: {e}", path.display()))?;
    Ok(path)
}

fn safe(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control() || c == '"' || c == '\\')
}

fn in_range(name: &str, v: u32, min: u32, max: u32) -> Result<(), String> {
    if v < min || v > max {
        return Err(format!("{name} deve estar entre {min} e {max}"));
    }
    Ok(())
}

fn validate_e4all_fields(c: &E4allConfig) -> Result<(), String> {
    if c.minecraft_dir.trim().is_empty() {
        return Err("Informe a pasta do Minecraft".into());
    }
    for (name, url) in [
        ("URL do broker", &c.broker_url),
        ("URL do mapa de relays", &c.dialtone_relay_map),
    ] {
        if !safe(url) || !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("{name} inválida (use http:// ou https://)"));
        }
    }
    if !safe(&c.relay_host) || c.relay_host.contains(char::is_whitespace) {
        return Err("Host do relay inválido".into());
    }
    in_range("Porta do relay", c.relay_port, 1, 65535)?;
    in_range("Tentativas de reconexão", c.reconnect_max_attempts, 0, 100)?;
    in_range(
        "Atraso inicial de reconexão (s)",
        c.reconnect_base_delay_seconds,
        1,
        3600,
    )?;
    in_range("Keepalive (s)", c.keepalive_interval_seconds, 1, 3600)?;
    in_range(
        "Tempo de espera do login (ms)",
        c.login_ready_timeout_ms,
        0,
        600_000,
    )?;
    in_range(
        "Tempo de conexão ao relay (ms)",
        c.relay_connect_timeout_ms,
        100,
        600_000,
    )?;
    Ok(())
}

// ───────────────────────── Detecção de loader (AUMENTADA) ─────────────────────────

fn classify(id: &str) -> &'static str {
    let l = id.to_lowercase();
    if l.contains("neoforge") {
        "neoforge"
    } else if l.contains("quilt") {
        "quilt"
    } else if l.contains("fabric") {
        "fabric"
    } else if l.contains("forge") {
        "forge"
    } else {
        "none"
    }
}

fn detect_loader(dir: &Path) -> (String, String, String) {
    // 1) Perfil usado no launcher oficial
    if let Ok(text) = fs::read_to_string(dir.join("launcher_profiles.json")) {
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            if let Some(profiles) = json.get("profiles").and_then(|p| p.as_object()) {
                let latest = profiles
                    .values()
                    .filter_map(|p| {
                        let id = p.get("lastVersionId")?.as_str()?;
                        let used = p.get("lastUsed").and_then(|u| u.as_str()).unwrap_or("");
                        Some((used.to_string(), id.to_string()))
                    })
                    .max_by(|a, b| a.0.cmp(&b.0));
                if let Some((_, id)) = latest {
                    let c = classify(&id);
                    if c != "none" {
                        return (c.to_string(), id, "perfil do launcher oficial".into());
                    }
                }
            }
        }
    }

    // 2) Pasta versions
    let mut found: Vec<(&'static str, String)> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir.join("versions")) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                let c = classify(&name);
                if c != "none" {
                    found.push((c, name));
                }
            }
        }
    }
    for pref in ["neoforge", "fabric", "forge", "quilt"] {
        if let Some((c, n)) = found.iter().find(|(c, _)| *c == pref) {
            return (c.to_string(), n.clone(), "pasta versions".into());
        }
    }

    // 3) Pasta libraries (procura em subpastas do net/fabricmc, net/minecraftforge, etc)
    for (rel, name) in [
        ("net/neoforged", "neoforge"),
        ("net/fabricmc", "fabric"),
        ("net/minecraftforge", "forge"),
        ("org/quiltmc", "quilt"),
    ] {
        if dir.join("libraries").join(rel).is_dir() {
            return (name.to_string(), String::new(), "pasta libraries".into());
        }
    }

    // 4) Fallback: ler o log inteiro (onde o loader sempre se anuncia)
    if let Ok(log) = fs::read_to_string(dir.join("logs").join("latest.log")) {
        let low = log.to_lowercase();
        if low.contains("neoforge") {
            return ("neoforge".into(), String::new(), "logs/latest.log".into());
        }
        if low.contains("quilt") {
            return ("quilt".into(), String::new(), "logs/latest.log".into());
        }
        if low.contains("fabric") {
            return ("fabric".into(), String::new(), "logs/latest.log".into());
        }
        if low.contains("forge") {
            return ("forge".into(), String::new(), "logs/latest.log".into());
        }
    }

    (
        "none".into(),
        String::new(),
        "nenhum mod loader encontrado".into(),
    )
}

#[derive(Serialize, Clone)]
struct E4allStatus {
    dir_exists: bool,
    loader: String,
    loader_id: String,
    loader_source: String,
    installed: bool,
    jar: String,
    config_exists: bool,
    config_path: String,
    level: String,
    message: String,
}

/// Procura recursivamente por qualquer .jar que contenha "e4all", "e4mc" ou "dialtone".
fn find_e4all_jar(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_e4all_jar(&path) {
                return Some(found);
            }
        } else if path.is_file() {
            let name = path.file_name()?.to_string_lossy().to_lowercase();
            if name.ends_with(".jar")
                && (name.contains("e4all") || name.contains("e4mc") || name.contains("dialtone"))
            {
                return Some(path);
            }
        }
    }
    None
}

fn detect_status(dir_str: &str) -> E4allStatus {
    let dir = expand(dir_str.trim());
    let cfg_path = dir.join("config").join("e4all.toml");
    let dir_exists = dir.is_dir();

    let mut st = E4allStatus {
        dir_exists,
        loader: "none".into(),
        loader_id: String::new(),
        loader_source: String::new(),
        installed: false,
        jar: String::new(),
        config_exists: cfg_path.is_file(),
        config_path: cfg_path.display().to_string(),
        level: "error".into(),
        message: String::new(),
    };

    if !dir_exists {
        st.message = format!("Pasta '{}' não encontrada. Se usa Prism/MultiMC, aponte para a pasta .minecraft da instância.", dir.display());
        return st;
    }

    let (loader, id, source) = detect_loader(&dir);
    st.loader = loader;
    st.loader_id = id;
    st.loader_source = source;

    if let Some(jar_path) = find_e4all_jar(&dir.join("mods")) {
        st.installed = true;
        st.jar = jar_path
            .strip_prefix(dir.join("mods"))
            .unwrap_or(&jar_path)
            .display()
            .to_string();
    }

    let loader_name = match st.loader.as_str() {
        "fabric" => "Fabric",
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        "quilt" => "Quilt",
        _ => "nenhum",
    };

    if st.loader == "none" {
        st.level = "error".into();
        st.message = "Mod loader não detectado. Se você usa um launcher como Prism ou CurseForge, use o botão 'Procurar launchers' para escolher a pasta correta.".into();
    } else if st.loader == "quilt" {
        st.level = "warn".into();
        st.message =
            "Quilt detectado. O e4all suporta oficialmente Fabric, Forge e NeoForge.".into();
    } else if !st.installed {
        st.level = "warn".into();
        st.message = format!("Mod loader ({loader_name}) OK, mas o e4all não foi achado na pasta mods/. Baixe o JAR no Modrinth e coloque lá.");
    } else {
        st.level = "ok".into();
        st.message = format!("Tudo certo! Mod loader ({loader_name}) e e4all detectados.");
    }
    st
}

// ───────────────────────── Auto-descoberta de launchers ─────────────────────────

#[derive(Serialize, Clone)]
struct LauncherCandidate {
    label: String,
    path: String,
    exists: bool,
    loader: String,
    e4all_installed: bool,
    config_exists: bool,
}

fn known_launcher_dirs() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));

    // 1. Launcher Oficial
    if home.join(".minecraft").is_dir() {
        out.push(("Launcher oficial", home.join(".minecraft")));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata).join(".minecraft");
        if p.is_dir() {
            out.push(("Launcher oficial (AppData)", p));
        }
    }

    // 2. Prism Launcher (instâncias)
    let prism_paths = [
        home.join(".local")
            .join("share")
            .join("PrismLauncher")
            .join("instances"),
        home.join("Library")
            .join("Application Support")
            .join("PrismLauncher")
            .join("instances"),
        PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
            .join("PrismLauncher")
            .join("instances"),
    ];
    for root in prism_paths {
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                let inst = e.path().join(".minecraft");
                if inst.is_dir() {
                    let label = format!("Prism: {}", e.file_name().to_string_lossy());
                    out.push((Box::leak(label.into_boxed_str()), inst));
                }
            }
        }
    }

    // 3. CurseForge (instâncias)
    let cf_paths = [
        home.join("Documents")
            .join("curseforge")
            .join("minecraft")
            .join("Instances"),
        home.join("curseforge").join("minecraft").join("Instances"),
    ];
    for root in cf_paths {
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                let inst = e.path();
                if inst.is_dir() {
                    let label = format!("CurseForge: {}", e.file_name().to_string_lossy());
                    out.push((Box::leak(label.into_boxed_str()), inst));
                }
            }
        }
    }
    out
}

fn scan_launchers() -> Vec<LauncherCandidate> {
    known_launcher_dirs()
        .into_iter()
        .map(|(label, path)| {
            let (loader, _, _) = detect_loader(&path);
            let e4all_installed = find_e4all_jar(&path.join("mods")).is_some();
            let config_exists = path.join("config").join("e4all.toml").is_file();
            LauncherCandidate {
                label: label.to_string(),
                path: path.display().to_string(),
                exists: true,
                loader,
                e4all_installed,
                config_exists,
            }
        })
        .collect()
}

async fn e4all_scan() -> Json<Vec<LauncherCandidate>> {
    Json(scan_launchers())
}

// ───────────────────────── Helpers de servidor ─────────────────────────

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

fn profiles_path(cfg_path: &Path) -> PathBuf {
    cfg_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("servers.json")
}

async fn load_profiles(cfg_path: &Path, current: &Config) -> Vec<Config> {
    let path = profiles_path(cfg_path);
    if let Ok(data) = tokio::fs::read_to_string(path).await {
        if let Ok(items) = serde_json::from_str::<Vec<Config>>(&data) {
            if !items.is_empty() {
                return items;
            }
        }
    }
    let current_dir = expand(&current.server_dir);
    if current_dir.join("server.jar").is_file()
        || current_dir.join("fabric-server-launch.jar").is_file()
        || current_dir.join("server.properties").is_file()
        || current_dir.join("eula.txt").is_file()
    {
        vec![current.clone()]
    } else {
        Vec::new()
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
    profiles: Vec<Config>,
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
    fn new(cfg: Config, profiles: Vec<Config>, cfg_path: PathBuf, java_detected: bool) -> Self {
        let mut s = Self {
            cfg,
            profiles,
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

    async fn persist(&mut self) -> std::io::Result<()> {
        if let Some(parent) = self.cfg_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let current_dir = self.cfg.server_dir.clone();
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.server_dir == current_dir)
        {
            *existing = self.cfg.clone();
        } else if self.cfg.server_name != Config::default().server_name
            || expand(&self.cfg.server_dir).join("server.jar").is_file()
            || expand(&self.cfg.server_dir)
                .join("fabric-server-launch.jar")
                .is_file()
        {
            self.profiles.push(self.cfg.clone());
        }
        let data = serde_json::to_vec_pretty(&self.cfg).map_err(std::io::Error::other)?;
        let temp = self.cfg_path.with_extension("json.tmp");
        tokio::fs::write(&temp, data).await?;
        tokio::fs::rename(temp, &self.cfg_path).await?;
        let profiles = serde_json::to_vec_pretty(&self.profiles).map_err(std::io::Error::other)?;
        let path = profiles_path(&self.cfg_path);
        let profiles_temp = path.with_extension("json.tmp");
        tokio::fs::write(&profiles_temp, profiles).await?;
        tokio::fs::rename(profiles_temp, path).await
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
    let installed =
        dir.join("server.jar").is_file() || dir.join("fabric-server-launch.jar").is_file();
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

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for code in chars.by_ref() {
                    if ('@'..='~').contains(&code) {
                        break;
                    }
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn log_kind(line: &str) -> &'static str {
    let lower = line.to_ascii_lowercase();
    if lower.contains("fatal")
        || lower.contains("error")
        || lower.contains("exception")
        || lower.contains("failed")
        || lower.contains("can't")
        || lower.contains("cannot")
    {
        "error"
    } else if lower.contains("warn") || lower.contains("deprecated") {
        "warning"
    } else if lower.contains("done (")
        || lower.contains("started")
        || lower.contains("stopping")
        || lower.contains("success")
    {
        "success"
    } else {
        "info"
    }
}

async fn pump<R: AsyncRead + Unpin>(app: AppState, reader: R) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(raw_line)) = lines.next_line().await {
        let line = strip_ansi(&raw_line);
        let mut i = app.0.lock().await;
        if line.contains(" joined the game") {
            i.players += 1;
        } else if line.contains(" left the game") {
            i.players = i.players.saturating_sub(1);
        }
        i.push(line.clone(), log_kind(&line));
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

#[derive(Serialize)]
struct ServerSummary {
    server_name: String,
    version: String,
    server_dir: String,
    installed: bool,
    active: bool,
    status: Status,
}

fn summary(profile: &Config, active_dir: &str, active_status: Status) -> ServerSummary {
    let dir = expand(&profile.server_dir);
    ServerSummary {
        server_name: profile.server_name.clone(),
        version: profile.version.clone(),
        server_dir: profile.server_dir.clone(),
        installed: dir.join("server.jar").is_file()
            || dir.join("fabric-server-launch.jar").is_file()
            || dir.join("server.properties").is_file(),
        active: profile.server_dir == active_dir,
        status: if profile.server_dir == active_dir {
            active_status
        } else {
            Status::Offline
        },
    }
}

async fn servers(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let active_dir = i.cfg.server_dir.clone();
    let active_status = i.status;
    let items: Vec<ServerSummary> = i
        .profiles
        .iter()
        .map(|profile| summary(profile, &active_dir, active_status))
        .collect();
    Ok(Json(json!({"servers": items})))
}

#[derive(Deserialize)]
struct SwitchServerRequest {
    server_dir: String,
}

async fn switch_server(
    State(app): State<AppState>,
    Json(req): Json<SwitchServerRequest>,
) -> ApiResult {
    let selected = {
        let i = app.0.lock().await;
        if i.status != Status::Offline {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor atual antes de trocar de servidor.",
            ));
        }
        i.profiles
            .iter()
            .find(|profile| profile.server_dir == req.server_dir)
            .cloned()
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Servidor não encontrado."))?
    };
    let java_detected = detect_java(&selected.java_path).await;
    let mut i = app.0.lock().await;
    if i.status != Status::Offline {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor atual antes de trocar de servidor.",
        ));
    }
    i.cfg = selected;
    i.java_detected = java_detected;
    i.logs.clear();
    i.players = 0;
    i.started = None;
    i.pid = None;
    i.stdin = None;
    i.kill_tx = None;
    let active_name = i.cfg.server_name.clone();
    i.push(format!("Servidor ativo: {active_name}"), "success");
    i.persist().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao trocar de servidor: {e}"),
        )
    })?;
    Ok(Json(view(&mut i)))
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

// ───────────────────────── e4all endpoints ─────────────────────────

#[derive(Deserialize)]
struct DirRequest {
    minecraft_dir: String,
}

async fn e4all_status(Json(req): Json<DirRequest>) -> Json<E4allStatus> {
    Json(detect_status(&req.minecraft_dir))
}

async fn e4all_load(Json(req): Json<DirRequest>) -> Json<E4allConfig> {
    Json(load_e4all(&req.minecraft_dir))
}

async fn save_e4all(State(app): State<AppState>, Json(mut cfg): Json<E4allConfig>) -> ApiResult {
    cfg.minecraft_dir = cfg.minecraft_dir.trim().into();
    cfg.broker_url = cfg.broker_url.trim().into();
    cfg.relay_host = cfg.relay_host.trim().into();
    cfg.dialtone_relay_map = cfg.dialtone_relay_map.trim().into();
    validate_e4all_fields(&cfg).map_err(|message| err(StatusCode::BAD_REQUEST, message))?;

    let written = match write_e4all_toml(&cfg) {
        Ok(p) => p,
        Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, e)),
    };

    let mut i = app.0.lock().await;
    i.cfg.e4all = cfg;
    if let Err(e) = i.persist().await {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao gravar config: {e}"),
        ));
    }
    i.push(
        format!(
            "e4all.toml atualizado em {}. Reabra o mundo LAN para aplicar.",
            written.display()
        ),
        "success",
    );
    Ok(Json(view(&mut i)))
}

// ───────────────────────── Delete / Command ─────────────────────────

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

    let deleted_dir = i.cfg.server_dir.clone();
    i.profiles
        .retain(|profile| profile.server_dir != deleted_dir);
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

// ───────────────────────── server.properties ─────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct ServerProperties {
    motd: String,
    max_players: u32,
    gamemode: String,
    difficulty: String,
    white_list: bool,
    online_mode: bool,
    allow_flight: bool,
    force_gamemode: bool,
    spawn_protection: u32,
    require_resource_pack: bool,
    resource_pack: String,
    resource_pack_prompt: String,
    view_distance: u32,
    simulation_distance: u32,
    pvp: bool,
    spawn_monsters: bool,
    spawn_animals: bool,
    generate_structures: bool,
    hardcore: bool,
    enable_command_block: bool,
    allow_nether: bool,
    level_name: String,
    level_seed: String,
    level_type: String,
    server_port: u16,
    icon_exists: bool,
    timezone: String,
    java_version: String,
}

impl Default for ServerProperties {
    fn default() -> Self {
        Self {
            motd: "Servidor ShadowPainel".into(),
            max_players: 20,
            gamemode: "survival".into(),
            difficulty: "easy".into(),
            white_list: false,
            online_mode: true,
            allow_flight: false,
            force_gamemode: false,
            spawn_protection: 0,
            require_resource_pack: false,
            resource_pack: "".into(),
            resource_pack_prompt: "".into(),
            view_distance: 10,
            simulation_distance: 10,
            pvp: true,
            spawn_monsters: true,
            spawn_animals: true,
            generate_structures: true,
            hardcore: false,
            enable_command_block: false,
            allow_nether: true,
            level_name: "world".into(),
            level_seed: "".into(),
            level_type: "minecraft:normal".into(),
            server_port: 25565,
            icon_exists: false,
            timezone: "America/Sao_Paulo".into(),
            java_version: "Java 21".into(),
        }
    }
}

async fn get_server_properties(
    State(app): State<AppState>,
) -> Result<Json<ServerProperties>, ApiErr> {
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    let path = dir.join("server.properties");

    let mut props = ServerProperties::default();

    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        let mut map = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        if let Some(v) = map.get("motd") {
            props.motd = v.clone();
        }
        if let Some(v) = map.get("max-players") {
            props.max_players = v.parse().unwrap_or(20);
        }
        if let Some(v) = map.get("gamemode") {
            props.gamemode = v.clone();
        }
        if let Some(v) = map.get("difficulty") {
            props.difficulty = v.clone();
        }
        if let Some(v) = map.get("white-list") {
            props.white_list = v == "true";
        }
        if let Some(v) = map.get("online-mode") {
            props.online_mode = v == "true";
        }
        if let Some(v) = map.get("allow-flight") {
            props.allow_flight = v == "true";
        }
        if let Some(v) = map.get("force-gamemode") {
            props.force_gamemode = v == "true";
        }
        if let Some(v) = map.get("spawn-protection") {
            props.spawn_protection = v.parse().unwrap_or(0);
        }
        if let Some(v) = map.get("require-resource-pack") {
            props.require_resource_pack = v == "true";
        }
        if let Some(v) = map.get("resource-pack") {
            props.resource_pack = v.clone();
        }
        if let Some(v) = map.get("resource-pack-prompt") {
            props.resource_pack_prompt = v.clone();
        }
        if let Some(v) = map.get("view-distance") {
            props.view_distance = v.parse().unwrap_or(10);
        }
        if let Some(v) = map.get("simulation-distance") {
            props.simulation_distance = v.parse().unwrap_or(10);
        }
        if let Some(v) = map.get("pvp") {
            props.pvp = v == "true";
        }
        if let Some(v) = map.get("spawn-monsters") {
            props.spawn_monsters = v == "true";
        }
        if let Some(v) = map.get("spawn-animals") {
            props.spawn_animals = v == "true";
        }
        if let Some(v) = map.get("generate-structures") {
            props.generate_structures = v == "true";
        }
        if let Some(v) = map.get("hardcore") {
            props.hardcore = v == "true";
        }
        if let Some(v) = map.get("enable-command-block") {
            props.enable_command_block = v == "true";
        }
        if let Some(v) = map.get("allow-nether") {
            props.allow_nether = v == "true";
        }
        if let Some(v) = map.get("level-name") {
            props.level_name = v.clone();
        }
        if let Some(v) = map.get("level-seed") {
            props.level_seed = v.clone();
        }
        if let Some(v) = map.get("level-type") {
            props.level_type = v.clone();
        }
        if let Some(v) = map.get("server-port") {
            props.server_port = v.parse().unwrap_or(25565);
        }
        props.icon_exists = dir.join("server-icon.png").is_file();
        if let Some(v) = map.get("shadowpainel-timezone") {
            props.timezone = v.clone();
        }
        if let Some(v) = map.get("shadowpainel-java-version") {
            props.java_version = v.clone();
        }
    }

    Ok(Json(props))
}

async fn save_server_properties(
    State(app): State<AppState>,
    Json(req): Json<ServerProperties>,
) -> Result<Json<Value>, ApiErr> {
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);

    if !dir.exists() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "A pasta do servidor não existe.",
        ));
    }

    let path = dir.join("server.properties");

    let mut lines = Vec::new();
    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        lines = content.lines().map(|s| s.to_string()).collect();
    } else {
        lines.push("#Minecraft server properties".into());
    }

    let mut map: HashMap<String, String> = HashMap::new();
    for line in &lines {
        if let Some((k, v)) = line.trim().split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    map.insert("motd".into(), req.motd);
    map.insert("max-players".into(), req.max_players.to_string());
    map.insert("gamemode".into(), req.gamemode);
    map.insert("difficulty".into(), req.difficulty);
    map.insert("white-list".into(), req.white_list.to_string());
    map.insert("online-mode".into(), req.online_mode.to_string());
    map.insert("allow-flight".into(), req.allow_flight.to_string());
    map.insert("force-gamemode".into(), req.force_gamemode.to_string());
    map.insert("spawn-protection".into(), req.spawn_protection.to_string());
    map.insert(
        "require-resource-pack".into(),
        req.require_resource_pack.to_string(),
    );
    map.insert("resource-pack".into(), req.resource_pack);
    map.insert("resource-pack-prompt".into(), req.resource_pack_prompt);
    map.insert("view-distance".into(), req.view_distance.to_string());
    map.insert(
        "simulation-distance".into(),
        req.simulation_distance.to_string(),
    );
    map.insert("pvp".into(), req.pvp.to_string());
    map.insert("spawn-monsters".into(), req.spawn_monsters.to_string());
    map.insert("spawn-animals".into(), req.spawn_animals.to_string());
    map.insert(
        "generate-structures".into(),
        req.generate_structures.to_string(),
    );
    map.insert("hardcore".into(), req.hardcore.to_string());
    map.insert(
        "enable-command-block".into(),
        req.enable_command_block.to_string(),
    );
    map.insert("allow-nether".into(), req.allow_nether.to_string());
    map.insert("level-name".into(), req.level_name);
    map.insert("level-seed".into(), req.level_seed);
    map.insert("level-type".into(), req.level_type);
    map.insert("server-port".into(), req.server_port.to_string());
    map.insert("shadowpainel-timezone".into(), req.timezone);
    map.insert("shadowpainel-java-version".into(), req.java_version);

    let mut out = String::new();
    for line in lines {
        if line.trim().starts_with('#') || line.trim().is_empty() {
            out.push_str(&line);
            out.push('\n');
        } else if let Some((k, _)) = line.trim().split_once('=') {
            let key = k.trim();
            if let Some(val) = map.remove(key) {
                out.push_str(&format!("{}={}\n", key, val));
            }
        }
    }
    for (k, v) in map {
        out.push_str(&format!("{}={}\n", k, v));
    }

    tokio::fs::write(&path, out).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar server.properties: {e}"),
        )
    })?;

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ServerIconRequest {
    data_url: String,
}

async fn save_server_icon(
    State(app): State<AppState>,
    Json(req): Json<ServerIconRequest>,
) -> Result<Json<Value>, ApiErr> {
    let encoded = req
        .data_url
        .split_once(',')
        .map(|(_, value)| value)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Imagem inválida."))?;
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "A imagem não está em PNG válido."))?;
    if bytes.len() > 1024 * 1024 {
        return Err(err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "O ícone não pode passar de 1 MB.",
        ));
    }
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(err(StatusCode::BAD_REQUEST, "O ícone precisa ser um PNG."));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width != 64 || height != 64 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "O ícone precisa ter exatamente 64×64 pixels.",
        ));
    }
    let i = app.0.lock().await;
    let dir = expand(&i.cfg.server_dir);
    if !dir.exists() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "A pasta do servidor não existe.",
        ));
    }
    tokio::fs::write(dir.join("server-icon.png"), bytes)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao salvar o ícone: {e}"),
            )
        })?;
    Ok(Json(
        json!({ "ok": true, "message": "server-icon.png salvo. Reinicie o servidor para aplicar." }),
    ))
}

// ───────────────────────── Mods / Fabric ─────────────────────────

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

// ───────────────────────── Conteúdo ─────────────────────────

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
    title: Option<String>,
    kind: String,
    project_id: Option<String>,
    version_number: Option<String>,
    icon_url: Option<String>,
    installed_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct InstalledContent {
    project_id: String,
    kind: String,
    filename: String,
    version_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}
fn content_manifest(base: &std::path::Path) -> PathBuf {
    base.join(".shadowpainel-content.json")
}
async fn read_installed_content(base: &std::path::Path) -> Vec<InstalledContent> {
    tokio::fs::read(content_manifest(base))
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
async fn enrich_installed_content(
    client: &reqwest::Client,
    items: &mut [InstalledContent],
) -> bool {
    let mut changed = false;
    let project_ids: Vec<String> = items
        .iter()
        .filter(|item| item.title.is_none() || item.icon_url.is_none())
        .map(|item| item.project_id.clone())
        .filter(|id| !id.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if !project_ids.is_empty() {
        if let Ok(ids) = serde_json::to_string(&project_ids) {
            let url = format!(
                "https://api.modrinth.com/v2/projects?ids={}",
                urlencoding::encode(&ids)
            );
            if let Ok(response) = client
                .get(url)
                .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
                .send()
                .await
            {
                if let Ok(projects) = response.json::<Vec<Value>>().await {
                    let by_id: HashMap<String, Value> = projects
                        .into_iter()
                        .filter_map(|project| {
                            Some((project.get("id")?.as_str()?.to_string(), project))
                        })
                        .collect();
                    for item in items.iter_mut() {
                        if let Some(project) = by_id.get(&item.project_id) {
                            let title = project
                                .get("title")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            let icon = project
                                .get("icon_url")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            if item.title != title || item.icon_url != icon {
                                item.title = title;
                                item.icon_url = icon;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    let version_ids: Vec<String> = items
        .iter()
        .filter(|item| item.version_number.is_none())
        .map(|item| item.version_id.clone())
        .filter(|id| !id.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    if !version_ids.is_empty() {
        if let Ok(ids) = serde_json::to_string(&version_ids) {
            let url = format!(
                "https://api.modrinth.com/v2/versions?ids={}",
                urlencoding::encode(&ids)
            );
            if let Ok(response) = client
                .get(url)
                .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
                .send()
                .await
            {
                if let Ok(versions) = response.json::<Vec<Value>>().await {
                    let by_id: HashMap<String, Value> = versions
                        .into_iter()
                        .filter_map(|version| {
                            Some((version.get("id")?.as_str()?.to_string(), version))
                        })
                        .collect();
                    for item in items.iter_mut() {
                        if let Some(version) = by_id.get(&item.version_id) {
                            let number = version
                                .get("version_number")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            if item.version_number != number {
                                item.version_number = number;
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }
    changed
}

async fn write_installed_content(
    base: &std::path::Path,
    items: &[InstalledContent],
) -> Result<(), ApiErr> {
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar catálogo: {e}"),
        )
    })?;
    tokio::fs::write(content_manifest(base), bytes)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao salvar catálogo: {e}"),
            )
        })
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
    let mut installed = read_installed_content(&base).await;
    let client = reqwest::Client::new();
    if enrich_installed_content(&client, &mut installed).await {
        let _ = write_installed_content(&base, &installed).await;
    }
    let metadata: HashMap<String, InstalledContent> = installed
        .into_iter()
        .filter(|item| item.kind == kind)
        .map(|item| (item.filename.clone(), item))
        .collect();
    let mut items = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao ler conteúdo: {e}"),
        )
    })? {
        let file_meta = entry.metadata().await.ok();
        if file_meta.as_ref().is_some_and(|m| m.is_file()) {
            let filename = entry.file_name().to_string_lossy().into_owned();
            if filename.starts_with('.') {
                continue;
            }
            let stored = metadata.get(&filename);
            let version_number = stored
                .and_then(|item| item.version_number.clone())
                .or_else(|| Some(filename.trim_end_matches(".jar").to_string()));

            items.push(ContentItem {
                filename,
                size: file_meta.unwrap().len(),
                title: stored.and_then(|item| item.title.clone()),
                kind: kind.clone(),
                project_id: stored.map(|item| item.project_id.clone()),
                version_number: version_number.clone(),
                icon_url: stored.and_then(|item| item.icon_url.clone()),
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

    let base = dir.parent().unwrap_or(&dir);
    let mut installed = read_installed_content(base).await;
    installed.retain(|x| {
        !deleted
            .iter()
            .any(|name| x.kind == req.kind && x.filename == *name)
    });
    write_installed_content(base, &installed).await?;
    if !errors.is_empty() {
        return Ok(Json(
            json!({"ok": false, "deleted": deleted, "errors": errors}),
        ));
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
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_content_limit")]
    limit: usize,
}

fn default_content_limit() -> usize {
    12
}

async fn search_content(
    State(app): State<AppState>,
    Json(req): Json<ContentSearchRequest>,
) -> Result<Json<Value>, ApiErr> {
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

    let limit = req.limit.clamp(1, 24);
    let url = format!(
        "https://api.modrinth.com/v2/search?query={}&facets={}&limit={}&offset={}",
        urlencoding::encode(&req.query),
        urlencoding::encode(&facets),
        limit,
        req.offset
    );

    let mut data: Value = reqwest::Client::new()
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
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    let installed: std::collections::HashSet<String> = read_installed_content(&base)
        .await
        .into_iter()
        .filter(|x| x.kind == req.kind)
        .map(|x| x.project_id)
        .collect();
    if let Some(hits) = data.get_mut("hits").and_then(Value::as_array_mut) {
        for hit in hits {
            let id = hit
                .get("project_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            hit["installed"] = Value::Bool(installed.contains(id));
        }
    }
    Ok(Json(data))
}

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
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao buscar versões: {e}"),
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

    Ok(Json(data))
}

#[derive(Deserialize)]
struct ContentInstallRequest {
    project_id: String,
    kind: String,
    loader: String,
    version: String,
    version_id: Option<String>,
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
    let url = if let Some(version_id) = req.version_id.as_deref().filter(|id| !id.is_empty()) {
        format!(
            "https://api.modrinth.com/v2/version/{}",
            urlencoding::encode(version_id)
        )
    } else {
        format!("https://api.modrinth.com/v2/project/{}/version?loaders=[\"{}\"]&game_versions=[\"{}\"]", urlencoding::encode(&req.project_id), urlencoding::encode(&req.loader), urlencoding::encode(&req.version))
    };
    let version_data: Value = client
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
    let versions: Vec<Value> = match version_data {
        Value::Array(items) => items,
        item => vec![item],
    };
    let version = versions.first();

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
    let mut installed = read_installed_content(&base).await;
    installed.retain(|x| !(x.kind == req.kind && x.filename == safe));
    let selected_version_id = version
        .and_then(|value| value.get("id").and_then(Value::as_str))
        .unwrap_or_else(|| req.version_id.as_deref().unwrap_or_default())
        .to_string();
    let selected_version_number = version
        .and_then(|value| value.get("version_number").and_then(Value::as_str))
        .map(str::to_owned);
    let mut record = InstalledContent {
        project_id: req.project_id,
        kind: req.kind,
        filename: safe.to_string(),
        version_id: selected_version_id,
        title: None,
        version_number: selected_version_number,
        icon_url: None,
    };
    enrich_installed_content(&client, std::slice::from_mut(&mut record)).await;
    installed.push(record);
    write_installed_content(&base, &installed).await?;
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
    let mut cfg = load_config(&cfg_path).await;

    let loaded = load_e4all(&cfg.e4all.minecraft_dir);
    cfg.e4all = loaded;

    let profiles = load_profiles(&cfg_path, &cfg).await;
    let java_detected = detect_java(&cfg.java_path).await;
    let app_state = AppState(Arc::new(Mutex::new(Inner::new(
        cfg,
        profiles,
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
        .route("/api/servers", get(servers))
        .route("/api/servers/switch", post(switch_server))
        .route("/api/server/start", post(start))
        .route("/api/server/stop", post(stop))
        .route("/api/server/delete", post(delete_server))
        .route("/api/server/download", post(download_server))
        .route("/api/server/eula", post(accept_eula))
        .route("/api/server/install-fabric", post(install_fabric))
        .route(
            "/api/server/properties",
            get(get_server_properties).post(save_server_properties),
        )
        .route("/api/server/icon", post(save_server_icon))
        .route("/api/minecraft/versions", get(minecraft_versions))
        .route("/api/server/create", post(create_server))
        .route("/api/content", get(list_content))
        .route("/api/content/delete", post(delete_content))
        .route("/api/content/search", post(search_content))
        .route("/api/content/versions", get(get_content_versions))
        .route("/api/content/install", post(install_content))
        .route("/api/mods/search", post(search_mods))
        .route("/api/mods/install", post(install_mod))
        .route("/api/config", post(save_config))
        .route("/api/e4all/config", post(save_e4all))
        .route("/api/e4all/status", post(e4all_status))
        .route("/api/e4all/load", post(e4all_load))
        .route("/api/e4all/scan", get(e4all_scan))
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
