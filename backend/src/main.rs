use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Local, Utc};
use md5::compute as md5_digest;
use qrcodegen::{QrCode, QrCodeEcc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::Read,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime},
};
use sysinfo::{Disks, Pid, ProcessesToUpdate, System};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    process::{Child, ChildStdin, Command},
    sync::{oneshot, Mutex, Semaphore},
    task::JoinSet,
};
use tower_http::services::ServeDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const APP_VERSION: &str = "0.8.3";

const MAX_LOGS: usize = 500;
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const PACK_DOWNLOAD_CONCURRENCY: usize = 4;
const PACK_INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

/// Id do Minecraft no CurseForge.
const CF_GAME_ID: &str = "432";

/// Classes usadas pelo catálogo do CurseForge.
const CF_CLASS_MODS: &str = "6";
const CF_CLASS_PLUGINS: &str = "5";
const CF_CLASS_MODPACKS: &str = "4471";

/// API oficial do CurseForge. Exige uma chave de API no cabeçalho `x-api-key`.
const CF_OFFICIAL_BASE: &str = "https://api.curseforge.com/v1";

/// Proxy público que expõe o mesmo contrato da API v1 do CurseForge, sem exigir chave.
const CF_PROXY_BASE: &str = "https://api.curse.tools/v1/cf";

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
    loader: String,
    server_dir: String,
    memory_limit_gb: u64,
    java_path: String,
    #[serde(default = "default_true")]
    auto_backup: bool,
    curseforge_api_key: String,
    e4all: E4allConfig,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let java_path = std::env::var("JAVA_HOME")
            .map(|p| format!("{p}/bin/java"))
            .unwrap_or_else(|_| "java".into());
        Self {
            server_name: "Sobrevivência principal".into(),
            version: "1.21.1".into(),
            loader: "vanilla".into(),
            server_dir: "~/Minecraft/MeuServidor".into(),
            memory_limit_gb: 4,
            java_path,
            auto_backup: true,
            curseforge_api_key: String::new(),
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

fn extract_e4all_link(line: &str) -> Option<String> {
    for marker in ["Domain reassigned: ", "Domain assigned: "] {
        let Some(index) = line.find(marker) else {
            continue;
        };
        let candidate = line[index + marker.len()..].split_whitespace().next()?;
        let candidate = candidate.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
        });
        if candidate.contains(".e4mc.link") {
            return Some(candidate.trim_end_matches('.').to_string());
        }
    }
    None
}

async fn latest_e4all_link(minecraft_dir: &str) -> Option<(String, bool)> {
    let path = expand(minecraft_dir.trim()).join("logs").join("latest.log");
    let modified = tokio::fs::metadata(&path).await.ok()?.modified().ok();
    let bytes = tokio::fs::read(path).await.ok()?;
    let start = bytes.len().saturating_sub(2 * 1024 * 1024);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let mut stopped = false;
    for line in text.lines().rev() {
        if line.contains("e4all: relay session stopped") {
            stopped = true;
            continue;
        }
        if let Some(link) = extract_e4all_link(line) {
            let recent = modified
                .and_then(|time| SystemTime::now().duration_since(time).ok())
                .is_some_and(|age| age < Duration::from_secs(300));
            return Some((link, !stopped && recent));
        }
    }
    None
}

fn qr_svg(data: &str) -> Option<String> {
    let qr = QrCode::encode_text(data, QrCodeEcc::Medium).ok()?;
    let border = 4;
    let size = qr.size() + border * 2;
    let mut path = String::new();
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + border, y + border));
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"{size}\" height=\"{size}\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#080a10\"/></svg>"
    ))
}

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

// ───────────────────────── Verificação do e4all.toml ─────────────────────────

#[derive(Serialize, Clone)]
struct E4allStatus {
    dir_exists: bool,
    config_exists: bool,
    config_path: String,
    level: String,
    message: String,
}

fn detect_status(dir_str: &str) -> E4allStatus {
    let dir = expand(dir_str.trim());
    let cfg_path = dir.join("config").join("e4all.toml");
    let dir_exists = dir.is_dir();
    let config_exists = cfg_path.is_file();
    let (level, message) = if !dir_exists {
        (
            "error",
            format!(
                "Pasta '{}' não encontrada. Aponte para a instância correta do Minecraft.",
                dir.display()
            ),
        )
    } else if config_exists {
        (
            "ok",
            "e4all.toml encontrado. As configurações podem ser lidas e salvas abaixo.".into(),
        )
    } else {
        (
            "warn",
            "e4all.toml não encontrado. Salve as configurações para criar o arquivo.".into(),
        )
    };
    E4allStatus {
        dir_exists,
        config_exists,
        config_path: cfg_path.display().to_string(),
        level: level.into(),
        message,
    }
}

// ───────────────────────── Auto-descoberta de launchers ─────────────────────────

#[derive(Serialize, Clone)]
struct LauncherCandidate {
    label: String,
    path: String,
    exists: bool,
    config_exists: bool,
}

fn known_launcher_dirs() -> Vec<(String, PathBuf)> {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut paths = Vec::new();
    let mut add = |label: String, path: PathBuf| {
        if path.is_dir() {
            paths.push((label, path));
        }
    };

    add("Launcher oficial".into(), home.join(".minecraft"));
    if let Ok(appdata) = std::env::var("APPDATA") {
        add(
            "Launcher oficial (AppData)".into(),
            PathBuf::from(appdata).join(".minecraft"),
        );
    }

    let prism_roots = [
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
    for root in prism_roots {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path().join(".minecraft");
                add(
                    format!("Prism: {}", entry.file_name().to_string_lossy()),
                    path,
                );
            }
        }
    }

    let curseforge_roots = [
        home.join("Documents")
            .join("curseforge")
            .join("minecraft")
            .join("Instances"),
        home.join("curseforge").join("minecraft").join("Instances"),
    ];
    for root in curseforge_roots {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                add(
                    format!("CurseForge: {}", entry.file_name().to_string_lossy()),
                    path,
                );
            }
        }
    }
    paths
}

fn scan_e4all_configs() -> Vec<LauncherCandidate> {
    known_launcher_dirs()
        .into_iter()
        .filter_map(|(label, path)| {
            let config_exists = path.join("config").join("e4all.toml").is_file();
            config_exists.then(|| LauncherCandidate {
                label: label.to_string(),
                path: path.display().to_string(),
                exists: true,
                config_exists,
            })
        })
        .collect()
}

async fn e4all_scan() -> Json<Vec<LauncherCandidate>> {
    Json(scan_e4all_configs())
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

fn java_major_for_version(version: &str) -> u32 {
    let parts: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    match parts.first() {
        Some(1) => {
            let minor = parts.get(1).copied().unwrap_or(21);
            let patch = parts.get(2).copied().unwrap_or(0);
            if minor >= 21 || (minor == 20 && patch >= 5) {
                21
            } else if minor >= 17 {
                17
            } else {
                8
            }
        }
        Some(year) if *year >= 26 => 25,
        _ => 21,
    }
}

fn java_runtimes_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".shadowpainel")
        .join("java")
}

fn find_java_binary(root: &Path) -> Option<PathBuf> {
    let wanted = if cfg!(windows) { "java.exe" } else { "java" };
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth > 8 || !dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push_back((path, depth + 1));
            } else if path.file_name().and_then(|name| name.to_str()) == Some(wanted)
                && path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some("bin")
            {
                return Some(path);
            }
        }
    }
    None
}

fn managed_java_path(major: u32) -> Option<PathBuf> {
    find_java_binary(&java_runtimes_dir().join(major.to_string()))
}

fn is_managed_java(path: &str) -> bool {
    expand(path).starts_with(java_runtimes_dir())
}

fn parse_java_major(output: &str) -> Option<u32> {
    let version = output.split("version \"").nth(1)?.split('"').next()?;
    let first = version.split('.').next()?.trim();
    if first == "1" {
        version.split('.').nth(1)?.parse().ok()
    } else {
        first.parse().ok()
    }
}

async fn detect_java_major(path: &str) -> Option<u32> {
    let mut command = Command::new(path);
    command.arg("-version");
    let output = tokio::time::timeout(Duration::from_secs(6), command.output())
        .await
        .ok()?
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    parse_java_major(&text)
}

fn adoptium_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

fn adoptium_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        "x86" => "x86",
        "arm" => "arm",
        other => other,
    }
}

async fn install_managed_java(major: u32) -> Result<PathBuf, ApiErr> {
    if let Some(path) = managed_java_path(major) {
        return Ok(path);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha no cliente HTTP: {e}"),
            )
        })?;
    let manifest_url = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={}&image_type=jre&os={}&vendor=eclipse",
        adoptium_arch(),
        adoptium_os()
    );
    let data: Value = client
        .get(&manifest_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao consultar o Java: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Adoptium retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Resposta inválida do Adoptium: {e}"),
            )
        })?;
    let package = data
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("binary"))
        .and_then(|binary| binary.get("package"))
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                format!("Java {major} não disponível para esta plataforma."),
            )
        })?;
    let link = package
        .get("link")
        .and_then(Value::as_str)
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Link do Java ausente."))?;
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("java-runtime.zip");
    let expected = package.get("checksum").and_then(Value::as_str);
    let bytes = client
        .get(link)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao baixar o Java: {e}"),
            )
        })?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Download do Java rejeitado: {e}"),
            )
        })?
        .bytes()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Falha ao ler o Java: {e}")))?;
    if let Some(expected) = expected {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "O arquivo do Java chegou corrompido. Tente novamente.",
            ));
        }
    }
    let root = java_runtimes_dir();
    tokio::fs::create_dir_all(&root).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a pasta do Java: {e}"),
        )
    })?;
    let archive = root.join(format!(".{major}-{name}"));
    let target = root.join(major.to_string());
    let _ = tokio::fs::remove_dir_all(&target).await;
    let _ = tokio::fs::remove_file(&archive).await;
    tokio::fs::write(&archive, &bytes).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar o Java: {e}"),
        )
    })?;
    tokio::fs::create_dir_all(&target).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar a pasta do Java: {e}"),
        )
    })?;
    let is_zip = name.ends_with(".zip");
    if is_zip {
        let archive_copy = archive.clone();
        let target_copy = target.clone();
        tokio::task::spawn_blocking(move || {
            extract_zip_blocking(archive_copy, target_copy, Vec::new())
        })
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao extrair o Java: {e}"),
            )
        })?
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&target);
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao extrair o Java: {e}"),
            )
        })?;
    } else {
        let status = tokio::time::timeout(
            Duration::from_secs(180),
            tokio::process::Command::new("tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&target)
                .status(),
        )
        .await
        .map_err(|_| {
            err(
                StatusCode::GATEWAY_TIMEOUT,
                "A extração do Java demorou demais. Tente novamente.",
            )
        })?
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao extrair o Java: {e}"),
            )
        })?;
        if !status.success() {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Falha ao extrair o arquivo do Java.",
            ));
        }
    }
    let _ = tokio::fs::remove_file(&archive).await;
    find_java_binary(&target).ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "O Java foi baixado, mas o executável não foi encontrado.",
        )
    })
}

fn eula_accepted(dir: &Path) -> bool {
    fs::read_to_string(dir.join("eula.txt"))
        .ok()
        .is_some_and(|contents| {
            contents
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("eula=true"))
        })
}

#[derive(Clone, Debug)]
enum Launch {
    Jar(String),
    Args(String),
}

fn first_dir_entry(dir: &Path, predicate: impl Fn(&str) -> bool) -> Option<String> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        (entry.path().is_dir() && predicate(&name)).then_some(name)
    })
}

fn detect_launch(dir: &Path) -> Option<Launch> {
    for jar in [
        "fabric-server-launch.jar",
        "quilt-server-launch.jar",
        "server.jar",
    ] {
        if dir.join(jar).is_file() {
            return Some(Launch::Jar(jar.to_string()));
        }
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_file()
                && name.starts_with("forge-")
                && name.ends_with(".jar")
                && !name.contains("installer")
                && !name.contains("sources")
            {
                return Some(Launch::Jar(name));
            }
        }
    }

    let args_file = if cfg!(windows) {
        "win_args.txt"
    } else {
        "unix_args.txt"
    };
    for root in [
        dir.join("libraries")
            .join("net")
            .join("minecraftforge")
            .join("forge"),
        dir.join("libraries")
            .join("net")
            .join("neoforged")
            .join("neoforge"),
        dir.join("libraries")
            .join("net")
            .join("neoforged")
            .join("forge"),
    ] {
        if let Some(version) = first_dir_entry(&root, |_| true) {
            let args = root.join(version).join(args_file);
            if args.is_file() {
                let relative = args
                    .strip_prefix(dir)
                    .unwrap_or(&args)
                    .to_string_lossy()
                    .into_owned();
                return Some(Launch::Args(relative));
            }
        }
    }
    None
}

fn infer_loader(dir: &Path) -> String {
    if dir.join("fabric-server-launch.jar").is_file() {
        return "fabric".into();
    }
    if dir.join("quilt-server-launch.jar").is_file() {
        return "quilt".into();
    }
    if let Some(Launch::Args(path)) = detect_launch(dir) {
        if path.contains("neoforge") {
            return "neoforge".into();
        }
        if path.contains("minecraftforge") {
            return "forge".into();
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("forge-") && name.ends_with(".jar") {
                return "forge".into();
            }
            if name.starts_with("paper-") || name == "paper.jar" {
                return "paper".into();
            }
            if name.starts_with("purpur-") || name == "purpur.jar" {
                return "purpur".into();
            }
        }
    }
    "vanilla".into()
}

fn safe_pack_path(base: &Path, relative: &str) -> Option<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return None;
    }
    let mut out = base.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (out != base).then_some(out)
}

fn safe_filename(filename: &str) -> Option<&str> {
    Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name == filename && !name.is_empty())
}

fn trash_root(base: &Path) -> PathBuf {
    base.join(".shadowpainel").join("trash")
}

fn new_trash_token() -> String {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".into())
}

fn valid_trash_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| c.is_ascii_digit())
}

fn valid_loader(loader: &str) -> bool {
    matches!(
        loader,
        "vanilla" | "fabric" | "quilt" | "forge" | "neoforge" | "paper" | "purpur"
    )
}

fn content_loader(kind: &str, loader: &str) -> String {
    match kind {
        "plugins" => match loader {
            "purpur" => "purpur".into(),
            _ => "paper".into(),
        },
        "datapacks" => "datapack".into(),
        _ => loader.into(),
    }
}

fn content_class_id(kind: &str) -> Result<&'static str, ApiErr> {
    match kind {
        "mods" => Ok(CF_CLASS_MODS),
        "plugins" => Ok(CF_CLASS_PLUGINS),
        "modpacks" => Ok(CF_CLASS_MODPACKS),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "Tipo de conteúdo inválido para o CurseForge.",
        )),
    }
}

fn cf_loader_type(loader: &str) -> Option<u8> {
    match loader {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

fn split_loader_id(id: &str) -> Option<(String, String)> {
    let (kind, version) = id.split_once('-')?;
    let kind = match kind.to_ascii_lowercase().as_str() {
        "forge" => "forge",
        "neoforge" => "neoforge",
        "fabric" => "fabric",
        "quilt" => "quilt",
        _ => return None,
    };
    Some((kind.into(), version.into()))
}

// ───────────────────────── Cliente CurseForge ─────────────────────────

#[derive(Clone)]
struct CurseForge {
    http: reqwest::Client,
    base: &'static str,
    key: Option<String>,
}

impl CurseForge {
    fn new(http: &reqwest::Client, key: &str) -> Self {
        let key = key.trim();
        if key.is_empty() {
            Self {
                http: http.clone(),
                base: CF_PROXY_BASE,
                key: None,
            }
        } else {
            Self {
                http: http.clone(),
                base: CF_OFFICIAL_BASE,
                key: Some(key.to_string()),
            }
        }
    }

    fn mode(&self) -> &'static str {
        if self.key.is_some() {
            "oficial"
        } else {
            "proxy"
        }
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, ApiErr> {
        let url = format!("{}{}", self.base, path);
        let mut request = self
            .http
            .get(url)
            .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
            .header("Accept", "application/json")
            .query(query);
        if let Some(key) = &self.key {
            request = request.header("x-api-key", key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("CurseForge: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(240).collect();
            return Err(err(
                StatusCode::BAD_GATEWAY,
                format!("CurseForge respondeu {status}. {detail}"),
            ));
        }
        response
            .json()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("CurseForge JSON: {e}")))
    }
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
    java_major: Option<u32>,
    status: Status,
    logs: VecDeque<LogLine>,
    players: u32,
    started: Option<DateTime<Utc>>,
    pid: Option<u32>,
    stdin: Option<ChildStdin>,
    kill_tx: Option<oneshot::Sender<()>>,
    installing: bool,
    e4all_link: Option<String>,
    sys: System,
}

impl Inner {
    fn new(
        cfg: Config,
        profiles: Vec<Config>,
        cfg_path: PathBuf,
        java_detected: bool,
        java_major: Option<u32>,
    ) -> Self {
        let mut s = Self {
            cfg,
            profiles,
            cfg_path,
            java_detected,
            java_major,
            status: Status::Offline,
            logs: VecDeque::new(),
            players: 0,
            started: None,
            pid: None,
            stdin: None,
            kill_tx: None,
            installing: false,
            e4all_link: None,
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
            || detect_launch(&expand(&self.cfg.server_dir)).is_some()
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

#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum HealthLevel {
    Ok,
    Warn,
    Danger,
}

#[derive(Serialize, Clone)]
struct HealthItem {
    id: String,
    label: String,
    level: HealthLevel,
    detail: String,
}

#[derive(Serialize, Clone)]
struct HealthView {
    level: HealthLevel,
    items: Vec<HealthItem>,
    advice: Vec<String>,
}

async fn set_java_path(app: &AppState, path: &Path, major: u32) -> Result<(), ApiErr> {
    let mut i = app.0.lock().await;
    i.cfg.java_path = path.to_string_lossy().into_owned();
    i.java_detected = true;
    i.java_major = Some(major);
    i.persist().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Java pronto, mas falhou ao salvar a configuração: {e}"),
        )
    })?;
    i.push(
        format!("Java {major} pronto em {}", path.display()),
        "success",
    );
    Ok(())
}

async fn ensure_java_for_version(app: &AppState, version: &str) -> Result<String, ApiErr> {
    let required = java_major_for_version(version);
    let current = {
        let i = app.0.lock().await;
        i.cfg.java_path.clone()
    };
    if detect_java_major(&current).await == Some(required) {
        let mut i = app.0.lock().await;
        i.java_detected = true;
        i.java_major = Some(required);
        return Ok(i.cfg.java_path.clone());
    }
    if let Some(path) = managed_java_path(required) {
        set_java_path(app, &path, required).await?;
        return Ok(path.to_string_lossy().into_owned());
    }
    let path = install_managed_java(required).await?;
    let detected = detect_java_major(&path.to_string_lossy()).await;
    let major = detected.unwrap_or(required);
    set_java_path(app, &path, major).await?;
    Ok(path.to_string_lossy().into_owned())
}

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
    loader: String,
    server_dir: String,
    java_path: String,
    java_detected: bool,
    java_major: Option<u32>,
    required_java_major: u32,
    java_managed: bool,
    installed: bool,
    eula_accepted: bool,
    installing: bool,
    auto_backup: bool,
    curseforge_mode: String,
    curseforge_configured: bool,
    e4all: E4allConfig,
    e4all_link: Option<String>,
    health: HealthView,
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
    let installed = detect_launch(&dir).is_some();
    let eula = eula_accepted(&dir);
    let uptime_seconds = i
        .started
        .map(|s| (Utc::now() - s).num_seconds().max(0) as u64)
        .unwrap_or(0);
    let address = format!("localhost:{}", server_port(&dir));
    let required_java_major = java_major_for_version(&i.cfg.version);

    let mut items = Vec::new();
    let mut advice = Vec::new();

    let java_ok = i.java_detected && i.java_major == Some(required_java_major);
    items.push(HealthItem {
        id: "java".into(),
        label: "Java".into(),
        level: if java_ok {
            HealthLevel::Ok
        } else {
            HealthLevel::Danger
        },
        detail: if java_ok {
            format!("Java {required_java_major} pronto.")
        } else if i.java_detected {
            format!(
                "Versão incompatível: detectado Java {}, necessário Java {required_java_major}.",
                i.java_major
                    .map(|major| major.to_string())
                    .unwrap_or_else(|| "desconhecido".into())
            )
        } else {
            format!("Java {required_java_major} não encontrado.")
        },
    });
    if !java_ok {
        advice.push("Use “Preparar Java automaticamente” para baixar a versão correta.".into());
    }

    i.sys.refresh_memory();
    let total_memory_mb = i.sys.total_memory() / 1024 / 1024;
    let limit_memory_mb = i.cfg.memory_limit_gb * 1024;
    let memory_percent = if limit_memory_mb > 0 {
        (memory_mb as f32 / limit_memory_mb as f32 * 100.0).min(999.0)
    } else {
        0.0
    };
    let memory_too_high =
        total_memory_mb > 0 && limit_memory_mb * 10 >= total_memory_mb.saturating_mul(8);
    let memory_too_low = i.cfg.memory_limit_gb < 2;
    let memory_level = if memory_percent >= 92.0 || memory_too_high {
        HealthLevel::Danger
    } else if memory_percent >= 78.0 || memory_too_low {
        HealthLevel::Warn
    } else {
        HealthLevel::Ok
    };
    items.push(HealthItem {
        id: "memory".into(),
        label: "Memória".into(),
        level: memory_level,
        detail: if i.status == Status::Offline {
            format!(
                "Limite: {} GB{}.",
                i.cfg.memory_limit_gb,
                if total_memory_mb > 0 {
                    format!(" · PC tem {:.1} GB", total_memory_mb as f64 / 1024.0)
                } else {
                    String::new()
                }
            )
        } else {
            format!(
                "{} MB de {} GB em uso ({:.0}%).",
                memory_mb, i.cfg.memory_limit_gb, memory_percent
            )
        },
    });
    if memory_too_high {
        advice.push(
            "O limite de RAM está alto demais para este PC. Reduza-o para não travar o sistema."
                .into(),
        );
    } else if memory_too_low {
        advice.push("Aumente a memória do servidor para pelo menos 2 GB.".into());
    } else if memory_percent >= 78.0 {
        advice.push(
            "O servidor está perto do limite de RAM. Reduza mods ou aumente o limite se houver RAM sobrando."
                .into(),
        );
    }

    let mut disk_available = None;
    let mut disk_total = None;
    for disk in Disks::new_with_refreshed_list().iter() {
        if dir.starts_with(disk.mount_point()) {
            disk_available = Some(disk.available_space());
            disk_total = Some(disk.total_space());
        }
    }
    let (disk_level, disk_detail) = match (disk_available, disk_total) {
        (Some(available), Some(total)) => {
            let gb = available as f64 / 1024.0 / 1024.0 / 1024.0;
            let level = if gb < 2.0 {
                HealthLevel::Danger
            } else if gb < 10.0 {
                HealthLevel::Warn
            } else {
                HealthLevel::Ok
            };
            (
                level,
                format!(
                    "{gb:.1} GB livres de {:.1} GB.",
                    total as f64 / 1024.0 / 1024.0 / 1024.0
                ),
            )
        }
        _ => (HealthLevel::Ok, "Espaço em disco não verificado.".into()),
    };
    items.push(HealthItem {
        id: "disk".into(),
        label: "Disco".into(),
        level: disk_level,
        detail: disk_detail,
    });
    if disk_level >= HealthLevel::Warn {
        advice.push("Libere espaço em disco antes de continuar.".into());
    }

    let max_players = std::fs::read_to_string(dir.join("server.properties"))
        .ok()
        .and_then(|text| property_from_text(&text, "max-players"))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(20)
        .max(1);
    let players_level = if i.status == Status::Online && i.players >= max_players {
        HealthLevel::Danger
    } else if i.status == Status::Online && i.players * 100 >= max_players * 80 {
        HealthLevel::Warn
    } else {
        HealthLevel::Ok
    };
    items.push(HealthItem {
        id: "players".into(),
        label: "Jogadores".into(),
        level: players_level,
        detail: if i.status == Status::Online {
            format!("{} de {} vagas ocupadas.", i.players, max_players)
        } else {
            format!("Servidor desligado · {} vagas configuradas.", max_players)
        },
    });
    if players_level >= HealthLevel::Warn {
        advice.push("O servidor está quase cheio: aumente as vagas ou entre mais tarde.".into());
    }

    let newest_backup = std::fs::read_dir(backups_dir(&dir))
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok()?.modified().ok())
                .max()
        });
    let backup_age_hours = newest_backup.and_then(|time| {
        SystemTime::now()
            .duration_since(time)
            .ok()
            .map(|age| age.as_secs() / 3600)
    });
    let backup_level = match backup_age_hours {
        None if installed => HealthLevel::Warn,
        Some(hours) if hours > 24 * 7 => HealthLevel::Warn,
        _ => HealthLevel::Ok,
    };
    items.push(HealthItem {
        id: "backup".into(),
        label: "Backup".into(),
        level: backup_level,
        detail: match backup_age_hours {
            Some(hours) if hours < 24 => format!("Último backup há {hours} h."),
            Some(hours) => format!("Último backup há {} dia(s).", hours / 24),
            None => "Nenhum backup encontrado.".into(),
        },
    });
    if backup_level == HealthLevel::Warn {
        advice.push("Crie um backup automático antes de instalar mods ou atualizar.".into());
    }

    let overall = items
        .iter()
        .map(|item| item.level)
        .max()
        .unwrap_or(HealthLevel::Ok);

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
        loader: i.cfg.loader.clone(),
        server_dir: i.cfg.server_dir.clone(),
        java_path: i.cfg.java_path.clone(),
        java_detected: i.java_detected,
        java_major: i.java_major,
        required_java_major,
        java_managed: is_managed_java(&i.cfg.java_path),
        installed,
        eula_accepted: eula,
        installing: i.installing,
        auto_backup: i.cfg.auto_backup,
        curseforge_mode: if i.cfg.curseforge_api_key.trim().is_empty() {
            "proxy".into()
        } else {
            "oficial".into()
        },
        curseforge_configured: !i.cfg.curseforge_api_key.trim().is_empty(),
        e4all: i.cfg.e4all.clone(),
        e4all_link: i.e4all_link.clone(),
        health: HealthView {
            level: overall,
            items,
            advice,
        },
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
        if let Some(link) = extract_e4all_link(&line) {
            i.e4all_link = Some(link);
        } else if line.contains("e4all: relay session stopped") {
            i.e4all_link = None;
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
    i.e4all_link = None;
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
        installed: detect_launch(&dir).is_some() || dir.join("server.properties").is_file(),
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
        if i.status != Status::Offline || i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor atual e aguarde instalações antes de trocar de servidor.",
            ));
        }
        i.profiles
            .iter()
            .find(|profile| profile.server_dir == req.server_dir)
            .cloned()
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Servidor não encontrado."))?
    };
    let java_major = detect_java_major(&selected.java_path).await;
    let java_detected = java_major.is_some();
    let mut i = app.0.lock().await;
    if i.status != Status::Offline || i.installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor atual e aguarde instalações antes de trocar de servidor.",
        ));
    }
    i.cfg = selected;
    i.java_detected = java_detected;
    i.java_major = java_major;
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
    if i.status != Status::Offline || i.installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor e aguarde instalações antes de baixar arquivos.",
        ));
    }
    let dir = expand(&i.cfg.server_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar pasta: {e}"),
        )
    })?;
    let jar_path = dir.join("server.jar");

    if detect_launch(&dir).is_some() || jar_path.exists() {
        return Err(err(
            StatusCode::CONFLICT,
            "Esta pasta já possui uma instalação de servidor",
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

#[derive(Deserialize)]
struct JavaEnsureRequest {
    #[serde(default)]
    version: Option<String>,
}

async fn java_ensure(State(app): State<AppState>, Json(req): Json<JavaEnsureRequest>) -> ApiResult {
    let version = {
        let i = app.0.lock().await;
        if i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Aguarde a instalação terminar antes de preparar o Java.",
            ));
        }
        req.version
            .filter(|version| !version.trim().is_empty())
            .unwrap_or_else(|| i.cfg.version.clone())
    };
    ensure_java_for_version(&app, &version).await?;
    let mut i = app.0.lock().await;
    Ok(Json(view(&mut i)))
}

async fn start(State(app): State<AppState>) -> ApiResult {
    let version = {
        let i = app.0.lock().await;
        if i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Aguarde a instalação terminar antes de iniciar.",
            ));
        }
        if i.status != Status::Offline {
            return Err(err(
                StatusCode::CONFLICT,
                "O servidor já está em execução ou parando.",
            ));
        }
        let dir = expand(&i.cfg.server_dir);
        if detect_launch(&dir).is_none() {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Nenhum servidor instalado nesta pasta. Use o assistente de criação.",
            ));
        }
        if !eula_accepted(&dir) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Aceite a EULA antes de iniciar.",
            ));
        }
        i.cfg.version.clone()
    };
    let java_path = ensure_java_for_version(&app, &version).await?;

    let mut i = app.0.lock().await;
    if i.installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Aguarde a instalação terminar antes de iniciar.",
        ));
    }
    if i.status != Status::Offline {
        return Err(err(
            StatusCode::CONFLICT,
            "O servidor já está em execução ou parando.",
        ));
    }
    let dir = expand(&i.cfg.server_dir);
    let launch = detect_launch(&dir).ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            "Nenhum servidor instalado nesta pasta. Use o assistente de criação.",
        )
    })?;
    if !eula_accepted(&dir) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Aceite a EULA antes de iniciar.",
        ));
    }

    let mut cmd = Command::new(&java_path);
    cmd.current_dir(&dir)
        .arg(format!("-Xmx{}G", i.cfg.memory_limit_gb))
        .arg("-Dfile.encoding=UTF-8");
    let launch_name = match launch {
        Launch::Jar(jar) => {
            cmd.args(["-jar", &jar, "nogui"]);
            jar
        }
        Launch::Args(args) => {
            cmd.arg(format!("@{args}")).arg("nogui");
            args
        }
    };

    cmd.stdin(Stdio::piped())
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
    i.e4all_link = None;
    i.push(format!("Iniciando servidor com {launch_name}"), "success");
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
    loader: Option<String>,
    server_dir: String,
    memory_limit_gb: u64,
    java_path: Option<String>,
    curseforge_api_key: Option<String>,
}

fn validate_config(req: &ConfigRequest) -> Result<(), String> {
    if req.server_name.trim().is_empty()
        || req.version.trim().is_empty()
        || req.server_dir.trim().is_empty()
    {
        return Err("Nome, versão e pasta são obrigatórios".into());
    }
    if let Some(loader) = req.loader.as_deref() {
        if !valid_loader(loader) {
            return Err("Tipo de servidor inválido.".into());
        }
    }
    if let Some(java_path) = req.java_path.as_deref() {
        if java_path.trim().is_empty() {
            return Err("Informe o caminho do Java.".into());
        }
    }
    if !(1..=64).contains(&req.memory_limit_gb) {
        return Err("Memória deve ficar entre 1 e 64 GB".into());
    }
    Ok(())
}

async fn save_config(State(app): State<AppState>, Json(req): Json<ConfigRequest>) -> ApiResult {
    validate_config(&req).map_err(|message| err(StatusCode::BAD_REQUEST, message))?;
    let mut i = app.0.lock().await;
    if i.status != Status::Offline || i.installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor e aguarde instalações para alterar a instalação.",
        ));
    }
    let java_path = req
        .java_path
        .as_deref()
        .map(str::trim)
        .unwrap_or(&i.cfg.java_path)
        .to_string();
    i.cfg.server_name = req.server_name.trim().into();
    i.cfg.version = req.version.trim().into();
    if let Some(loader) = req.loader.as_deref() {
        i.cfg.loader = loader.trim().into();
    }
    i.cfg.server_dir = req.server_dir.trim().into();
    i.cfg.memory_limit_gb = req.memory_limit_gb;
    if java_path != i.cfg.java_path {
        i.cfg.java_path = java_path;
        i.java_major = detect_java_major(&i.cfg.java_path).await;
        i.java_detected = i.java_major.is_some();
    }
    if let Some(key) = req.curseforge_api_key.as_deref() {
        let key = key.trim();
        if key.chars().any(char::is_control) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "A chave do CurseForge contém caracteres inválidos.",
            ));
        }
        i.cfg.curseforge_api_key = key.to_string();
    }
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

#[derive(Serialize)]
struct E4allLinkView {
    link: Option<String>,
    source: Option<String>,
    active: bool,
    checked_at: String,
    qr_svg: Option<String>,
}

async fn e4all_link(
    State(app): State<AppState>,
    Json(req): Json<DirRequest>,
) -> Json<E4allLinkView> {
    let (server_link, server_online, default_dir) = {
        let i = app.0.lock().await;
        (
            i.e4all_link.clone(),
            i.status == Status::Online,
            i.cfg.e4all.minecraft_dir.clone(),
        )
    };
    if let Some(link) = server_link {
        return Json(E4allLinkView {
            qr_svg: qr_svg(&link),
            link: Some(link),
            source: Some("servidor".into()),
            active: server_online,
            checked_at: Local::now().to_rfc3339(),
        });
    }
    let minecraft_dir = if req.minecraft_dir.trim().is_empty() {
        default_dir
    } else {
        req.minecraft_dir
    };
    let (link, active) = match latest_e4all_link(&minecraft_dir).await {
        Some((link, active)) => (Some(link), active),
        None => (None, false),
    };
    Json(E4allLinkView {
        qr_svg: link.as_deref().and_then(qr_svg),
        source: link.as_ref().map(|_| "Minecraft".into()),
        link,
        active,
        checked_at: Local::now().to_rfc3339(),
    })
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

// ───────────────────────── Rede / pastas ─────────────────────────

#[derive(Serialize)]
struct NetworkCheck {
    id: String,
    label: String,
    level: HealthLevel,
    detail: String,
}

#[derive(Serialize)]
struct NetworkDiagnostic {
    port: u16,
    local_ip: Option<String>,
    public_ip: Option<String>,
    checks: Vec<NetworkCheck>,
    summary: String,
    recommendation: String,
}

async fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(4),
        Command::new(program).args(args).output(),
    )
    .await
    .ok()?
    .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_lowercase();
    text.push_str(&String::from_utf8_lossy(&output.stderr).to_lowercase());
    Some(text)
}

async fn local_ipv4() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect("8.8.8.8:80").await.ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

async fn public_ipv4() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let text = client
        .get("https://api.ipify.org")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .await
        .ok()?;
    text.trim()
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

async fn firewall_check(port: u16) -> (HealthLevel, String) {
    if cfg!(windows) {
        let Some(text) = command_text("netsh", &["advfirewall", "show", "allprofiles"]).await
        else {
            return (
                HealthLevel::Warn,
                "Não foi possível consultar o Firewall do Windows.".into(),
            );
        };
        if text.contains("state") && text.contains("off") && !text.contains("on") {
            return (HealthLevel::Ok, "Firewall do Windows desativado.".into());
        }
        return (
            HealthLevel::Warn,
            format!("Firewall do Windows ativo: confirme a regra de entrada para a porta {port}."),
        );
    }
    if cfg!(target_os = "macos") {
        let Some(text) = command_text(
            "/usr/libexec/ApplicationFirewall/socketfilterfw",
            &["--getglobalstate"],
        )
        .await
        else {
            return (
                HealthLevel::Warn,
                "Não foi possível consultar o firewall do macOS.".into(),
            );
        };
        if text.contains("disabled") {
            return (HealthLevel::Ok, "Firewall do macOS desativado.".into());
        }
        return (
            HealthLevel::Warn,
            format!("Firewall do macOS ativo: permita o Java na porta {port} se necessário."),
        );
    }
    if let Some(text) = command_text("ufw", &["status"]).await {
        if text.contains("status: inactive") {
            return (HealthLevel::Ok, "UFW desativado.".into());
        }
        if text.contains("status: active") {
            let port_text = port.to_string();
            let allowed = text
                .lines()
                .any(|line| line.contains(&port_text) && line.contains("allow"));
            return if allowed {
                (
                    HealthLevel::Ok,
                    format!("UFW ativo com regra para a porta {port}."),
                )
            } else {
                (
                    HealthLevel::Warn,
                    format!("UFW ativo: libere a porta {port}/tcp para receber amigos."),
                )
            };
        }
    }
    if let Some(text) = command_text("firewall-cmd", &["--state"]).await {
        return if text.contains("running") {
            (
                HealthLevel::Warn,
                format!("firewalld ativo: confirme a porta {port}/tcp."),
            )
        } else {
            (HealthLevel::Ok, "firewalld desativado.".into())
        };
    }
    (
        HealthLevel::Ok,
        "Nenhum firewall conhecido detectado.".into(),
    )
}

async fn upnp_available() -> bool {
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await else {
        return false;
    };
    let message = concat!(
        "M-SEARCH * HTTP/1.1\r\n",
        "HOST: 239.255.255.250:1900\r\n",
        "MAN: \"ssdp:discover\"\r\n",
        "MX: 2\r\n",
        "ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n",
        "\r\n"
    );
    if socket
        .send_to(message.as_bytes(), "239.255.255.250:1900")
        .await
        .is_err()
    {
        return false;
    }
    let mut buffer = [0u8; 4096];
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.recv_from(&mut buffer).await {
                Ok((size, _)) => {
                    let text = String::from_utf8_lossy(&buffer[..size]).to_lowercase();
                    if text.contains("internetgatewaydevice") || text.contains("upnp") {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

async fn network_diagnose(State(app): State<AppState>) -> Json<NetworkDiagnostic> {
    let (dir, minecraft_dir, online) = {
        let i = app.0.lock().await;
        (
            expand(&i.cfg.server_dir),
            i.cfg.e4all.minecraft_dir.clone(),
            i.status == Status::Online,
        )
    };
    let port = server_port(&dir);
    let (local_ip, public_ip, (firewall_level, firewall_detail), upnp) = tokio::join!(
        local_ipv4(),
        public_ipv4(),
        firewall_check(port),
        upnp_available()
    );

    let (port_level, port_detail) = if online {
        match tokio::time::timeout(
            Duration::from_secs(2),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            Ok(Ok(_)) => (
                HealthLevel::Ok,
                format!("O servidor está respondendo na porta {port}."),
            ),
            _ => (
                HealthLevel::Danger,
                format!("O servidor está ligado, mas não responde na porta {port}."),
            ),
        }
    } else {
        match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(_) => (
                HealthLevel::Warn,
                format!("A porta {port} está livre, mas o servidor está desligado."),
            ),
            Err(_) => (
                HealthLevel::Danger,
                format!("A porta {port} já está ocupada por outro programa."),
            ),
        }
    };

    let internet_level = if public_ip.is_some() {
        HealthLevel::Ok
    } else {
        HealthLevel::Danger
    };
    let internet_detail = public_ip
        .as_ref()
        .map(|ip| format!("Seu IP público é {ip}."))
        .unwrap_or_else(|| "Não foi possível consultar seu IP público.".into());
    let upnp_level = if upnp {
        HealthLevel::Ok
    } else {
        HealthLevel::Warn
    };
    let upnp_detail = if upnp {
        "Roteador compatível com UPnP encontrado.".to_string()
    } else {
        "UPnP não encontrado. O e4all evita abrir portas no roteador.".to_string()
    };
    let (e4all_level, e4all_detail) = match latest_e4all_link(&minecraft_dir).await {
        Some((link, true)) if online => {
            (HealthLevel::Ok, format!("Link e4all ativo agora: {link}."))
        }
        Some((link, true)) => (
            HealthLevel::Warn,
            format!("O último link e4all foi {link}, mas o servidor está desligado."),
        ),
        Some((link, false)) => (
            HealthLevel::Warn,
            format!("O último link e4all ({link}) expirou. Inicie o servidor para gerar outro."),
        ),
        None => (
            HealthLevel::Warn,
            "Nenhum link e4all encontrado ainda. Ele aparece quando o servidor abre o mod."
                .to_string(),
        ),
    };
    let recommendation = if e4all_level == HealthLevel::Ok {
        "Compartilhe o link e4all ativo: ele funciona sem abrir portas no roteador."
    } else {
        "Inicie o servidor e compartilhe o link e4all para jogar sem abrir portas."
    };

    let summary = if !online {
        "O servidor está desligado. Ligue-o antes de testar com amigos.".to_string()
    } else if port_level == HealthLevel::Ok {
        "O servidor está escutando. O próximo passo é testar de outra rede.".to_string()
    } else {
        "O servidor está ligado, mas a porta não respondeu. Veja as dicas abaixo.".to_string()
    };

    Json(NetworkDiagnostic {
        port,
        local_ip,
        public_ip,
        checks: vec![
            NetworkCheck {
                id: "port".into(),
                label: "Porta do servidor".into(),
                level: port_level,
                detail: port_detail,
            },
            NetworkCheck {
                id: "firewall".into(),
                label: "Firewall".into(),
                level: firewall_level,
                detail: firewall_detail,
            },
            NetworkCheck {
                id: "upnp".into(),
                label: "Roteador / UPnP".into(),
                level: upnp_level,
                detail: upnp_detail,
            },
            NetworkCheck {
                id: "internet".into(),
                label: "Internet".into(),
                level: internet_level,
                detail: internet_detail,
            },
            NetworkCheck {
                id: "e4all".into(),
                label: "Alternativa e4all".into(),
                level: e4all_level,
                detail: e4all_detail,
            },
        ],
        summary,
        recommendation: recommendation.into(),
    })
}

async fn open_server_folder(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let dir = expand(&app.0.lock().await.cfg.server_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar a pasta do servidor: {e}"),
        )
    })?;
    let mut command = if cfg!(windows) {
        let mut command = Command::new("explorer");
        command.arg(&dir);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(&dir);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(&dir);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Não foi possível abrir a pasta: {e}"),
            )
        })?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct NextPortRequest {
    #[serde(default)]
    current: Option<u16>,
}

async fn next_port(Json(req): Json<NextPortRequest>) -> Json<Value> {
    let start = req.current.unwrap_or(25565).clamp(1024, 65000);
    for port in (start + 1)..=(start + 40) {
        if let Ok(listener) = TcpListener::bind(("0.0.0.0", port)).await {
            drop(listener);
            return Json(json!({ "port": port }));
        }
    }
    Json(json!({ "port": null }))
}

// ───────────────────────── Whitelist ─────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct WhitelistEntry {
    uuid: String,
    name: String,
}

fn whitelist_path(base: &Path) -> PathBuf {
    base.join("whitelist.json")
}

async fn read_whitelist(base: &Path) -> Vec<WhitelistEntry> {
    tokio::fs::read(whitelist_path(base))
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

async fn write_whitelist(base: &Path, entries: &[WhitelistEntry]) -> Result<(), ApiErr> {
    let bytes = serde_json::to_vec_pretty(entries).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar whitelist: {e}"),
        )
    })?;
    tokio::fs::write(whitelist_path(base), bytes)
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao salvar whitelist: {e}"),
            )
        })
}

fn property_from_text(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (line_key, value) = line.split_once('=')?;
        (line_key.trim() == key).then(|| value.trim().to_string())
    })
}

async fn set_server_property(base: &Path, key: &str, value: &str) -> Result<(), ApiErr> {
    let path = base.join("server.properties");
    let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let mut found = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        if let Some((line_key, _)) = line.split_once('=') {
            if line_key.trim() == key {
                lines.push(format!("{key}={value}"));
                found = true;
                continue;
            }
        }
        lines.push(line.to_string());
    }
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let mut output = lines.join("\n");
    output.push('\n');
    tokio::fs::write(path, output).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar server.properties: {e}"),
        )
    })
}

fn offline_uuid(name: &str) -> String {
    let digest = md5_digest(format!("OfflinePlayer:{name}").as_bytes());
    let mut bytes = digest.0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn valid_player_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

async fn whitelist_view(base: &Path) -> Result<Value, ApiErr> {
    let entries = read_whitelist(base).await;
    let props = tokio::fs::read_to_string(base.join("server.properties"))
        .await
        .unwrap_or_default();
    let enabled = property_from_text(&props, "white-list")
        .map(|value| value == "true")
        .unwrap_or(false);
    let online_mode = property_from_text(&props, "online-mode")
        .map(|value| value == "true")
        .unwrap_or(true);
    Ok(json!({
        "entries": entries,
        "enabled": enabled,
        "online_mode": online_mode,
    }))
}

async fn get_whitelist(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    Ok(Json(whitelist_view(&base).await?))
}

#[derive(Deserialize)]
struct WhitelistAddRequest {
    name: String,
    #[serde(default)]
    online_mode: Option<bool>,
}

async fn add_whitelist(
    State(app): State<AppState>,
    Json(req): Json<WhitelistAddRequest>,
) -> Result<Json<Value>, ApiErr> {
    let name = req.name.trim();
    if !valid_player_name(name) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Use um nome de jogador válido (3 a 16 letras, números ou _).",
        ));
    }
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    let props = tokio::fs::read_to_string(base.join("server.properties"))
        .await
        .unwrap_or_default();
    let online_mode = req.online_mode.unwrap_or_else(|| {
        property_from_text(&props, "online-mode")
            .map(|value| value == "true")
            .unwrap_or(true)
    });
    let entry = if online_mode {
        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://api.mojang.com/users/profiles/minecraft/{name}"
            ))
            .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
            .send()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Mojang: {e}")))?;
        if response.status() == StatusCode::NO_CONTENT || response.status() == StatusCode::NOT_FOUND
        {
            return Err(err(
                StatusCode::NOT_FOUND,
                "Jogador não encontrado. Confira o nome ou use a conta offline.",
            ));
        }
        let data: Value = response
            .error_for_status()
            .map_err(|e| {
                err(
                    StatusCode::BAD_GATEWAY,
                    format!("Mojang retornou erro: {e}"),
                )
            })?
            .json()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Mojang JSON: {e}")))?;
        let uuid = data["id"]
            .as_str()
            .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Mojang não retornou o UUID."))?;
        let canonical = data["name"].as_str().unwrap_or(name);
        WhitelistEntry {
            uuid: uuid.to_string(),
            name: canonical.to_string(),
        }
    } else {
        WhitelistEntry {
            uuid: offline_uuid(name),
            name: name.to_string(),
        }
    };

    let mut entries = read_whitelist(&base).await;
    entries.retain(|item| !item.name.eq_ignore_ascii_case(&entry.name));
    entries.push(entry.clone());
    write_whitelist(&base, &entries).await?;
    set_server_property(&base, "white-list", "true").await?;
    let mut i = app.0.lock().await;
    if i.status == Status::Online {
        let _ = send_line(&mut i, "whitelist reload").await;
        let _ = send_line(&mut i, "whitelist on").await;
    }
    i.push(
        format!(
            "{} convidado. UUID: {}",
            entry.name,
            if online_mode {
                "conta original"
            } else {
                "conta offline"
            }
        ),
        "success",
    );
    drop(i);
    Ok(Json(whitelist_view(&base).await?))
}

#[derive(Deserialize)]
struct WhitelistRemoveRequest {
    name: String,
}

async fn remove_whitelist(
    State(app): State<AppState>,
    Json(req): Json<WhitelistRemoveRequest>,
) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    let mut entries = read_whitelist(&base).await;
    let before = entries.len();
    entries.retain(|item| !item.name.eq_ignore_ascii_case(req.name.trim()));
    if entries.len() == before {
        return Err(err(StatusCode::NOT_FOUND, "Jogador não está na whitelist."));
    }
    write_whitelist(&base, &entries).await?;
    let mut i = app.0.lock().await;
    if i.status == Status::Online {
        let _ = send_line(&mut i, "whitelist reload").await;
    }
    i.push(
        format!("{} removido da whitelist.", req.name.trim()),
        "info",
    );
    drop(i);
    Ok(Json(whitelist_view(&base).await?))
}

#[derive(Deserialize)]
struct WhitelistToggleRequest {
    enabled: bool,
}

async fn toggle_whitelist(
    State(app): State<AppState>,
    Json(req): Json<WhitelistToggleRequest>,
) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    set_server_property(
        &base,
        "white-list",
        if req.enabled { "true" } else { "false" },
    )
    .await?;
    let mut i = app.0.lock().await;
    if i.status == Status::Online {
        let _ = send_line(
            &mut i,
            if req.enabled {
                "whitelist on"
            } else {
                "whitelist off"
            },
        )
        .await;
    }
    i.push(
        if req.enabled {
            "Whitelist ativada: só jogadores convidados entram."
        } else {
            "Whitelist desativada: qualquer jogador pode entrar."
        },
        "info",
    );
    drop(i);
    Ok(Json(whitelist_view(&base).await?))
}

// ───────────────────────── Atualizações ─────────────────────────

const RELEASES_API: &str = "https://api.github.com/repos/NoteDarck/ShadowPainel/releases";

fn updates_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".shadowpainel")
        .join("updates")
}

fn update_state_path() -> PathBuf {
    updates_dir().join("state.json")
}

#[cfg(unix)]
async fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await;
}

#[cfg(not(unix))]
async fn make_executable(_path: &Path) {}

fn version_parts(version: &str) -> (u64, u64, u64) {
    let mut parts = version.trim_start_matches('v').split('.').map(|part| {
        part.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn update_asset_rank(name: &str) -> Option<u8> {
    let lower = name.to_lowercase();
    if cfg!(windows) {
        if lower.ends_with(".exe") {
            return Some(0);
        }
        if lower.ends_with(".msi") {
            return Some(1);
        }
        return None;
    }
    if cfg!(target_os = "macos") {
        if lower.ends_with(".dmg") {
            return Some(0);
        }
        if lower.ends_with(".tar.gz") {
            return Some(1);
        }
        return None;
    }
    if lower.ends_with(".appimage") {
        Some(0)
    } else if lower.ends_with(".deb") {
        Some(1)
    } else if lower.ends_with(".tar.gz") {
        Some(2)
    } else {
        None
    }
}

fn release_asset(release: &Value) -> Option<(String, String, u64, Option<String>)> {
    release["assets"]
        .as_array()?
        .iter()
        .filter_map(|asset| {
            let name = asset["name"].as_str()?;
            let rank = update_asset_rank(name)?;
            let url = asset["browser_download_url"].as_str()?.to_string();
            let size = asset["size"].as_u64().unwrap_or(0);
            let digest = asset["digest"]
                .as_str()
                .and_then(|value| value.strip_prefix("sha256:"))
                .map(str::to_owned);
            Some((rank, name.to_string(), url, size, digest))
        })
        .min_by_key(|(rank, ..)| *rank)
        .map(|(_, name, url, size, digest)| (name, url, size, digest))
}

#[derive(Serialize)]
struct UpdateInfo {
    current_version: String,
    latest_version: String,
    available: bool,
    release_url: String,
    notes: String,
    published_at: String,
    asset_name: Option<String>,
    asset_size: u64,
    can_rollback: bool,
}

async fn fetch_release(client: &reqwest::Client, tag: Option<&str>) -> Result<Value, ApiErr> {
    let url = match tag {
        Some(tag) => format!("{RELEASES_API}/tags/{tag}"),
        None => format!("{RELEASES_API}/latest"),
    };
    client
        .get(url)
        .timeout(Duration::from_secs(20))
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("GitHub: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("GitHub retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("GitHub JSON: {e}")))
}

fn update_info(release: &Value) -> UpdateInfo {
    let latest = release["tag_name"]
        .as_str()
        .unwrap_or(APP_VERSION)
        .trim_start_matches('v')
        .to_string();
    let asset = release_asset(release);
    let previous = std::fs::read(update_state_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|state| state["previous_version"].as_str().map(str::to_owned));
    UpdateInfo {
        current_version: APP_VERSION.into(),
        available: version_parts(&latest) > version_parts(APP_VERSION),
        latest_version: latest,
        release_url: release["html_url"]
            .as_str()
            .unwrap_or("https://github.com/NoteDarck/ShadowPainel/releases")
            .into(),
        notes: release["body"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(4000)
            .collect(),
        published_at: release["published_at"].as_str().unwrap_or("").into(),
        asset_name: asset.as_ref().map(|(name, ..)| name.clone()),
        asset_size: asset.as_ref().map(|(_, _, size, _)| *size).unwrap_or(0),
        can_rollback: previous.is_some(),
    }
}

async fn check_update() -> Result<Json<UpdateInfo>, ApiErr> {
    let client = reqwest::Client::new();
    let release = fetch_release(&client, None).await?;
    Ok(Json(update_info(&release)))
}

fn update_checksum_from_sums(text: &str, asset_name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset_name).then(|| hash.to_lowercase())
    })
}

async fn install_update(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao preparar o download: {e}"),
            )
        })?;
    let release = fetch_release(&client, None).await?;
    let info = update_info(&release);
    if !info.available {
        return Err(err(
            StatusCode::CONFLICT,
            "Você já está usando a versão mais recente.",
        ));
    }
    let (asset_name, asset_url, _size, digest) = release_asset(&release).ok_or_else(|| {
        err(
            StatusCode::NOT_FOUND,
            "Esta versão não tem instalador para o seu sistema.",
        )
    })?;
    let dir = updates_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar atualizações: {e}"),
        )
    })?;
    let destination = dir.join(
        safe_filename(&asset_name)
            .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Nome de instalador inválido."))?,
    );
    if !destination.is_file() {
        save_download(&client, &asset_url, &destination).await?;
    }
    let expected = if let Some(digest) = digest {
        Some(digest)
    } else if let Some(url) = release["assets"].as_array().and_then(|assets| {
        assets.iter().find_map(|asset| {
            (asset["name"].as_str() == Some("SHA256SUMS.txt"))
                .then(|| asset["browser_download_url"].as_str())
                .flatten()
        })
    }) {
        let sums = client
            .get(url)
            .timeout(Duration::from_secs(20))
            .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
            .send()
            .await
            .ok()
            .and_then(|response| response.error_for_status().ok());
        match sums {
            Some(response) => response
                .text()
                .await
                .ok()
                .and_then(|text| update_checksum_from_sums(&text, &asset_name)),
            None => None,
        }
    } else {
        None
    };
    if let Some(expected) = expected {
        let bytes = tokio::fs::read(&destination).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao verificar o instalador: {e}"),
            )
        })?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&expected) {
            let _ = tokio::fs::remove_file(&destination).await;
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "O instalador chegou corrompido e foi apagado. Tente novamente.",
            ));
        }
    }
    make_executable(&destination).await;
    if let Some(parent) = config_path().parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let config = config_path();
    if config.is_file() {
        let backup = dir.join(format!("config-antes-de-{APP_VERSION}.json"));
        let _ = tokio::fs::copy(&config, &backup).await;
    }
    let state = json!({
        "previous_version": APP_VERSION,
        "target_version": info.latest_version,
        "asset": asset_name,
        "saved_at": Local::now().to_rfc3339(),
    });
    let _ = tokio::fs::write(
        update_state_path(),
        serde_json::to_vec_pretty(&state).unwrap_or_default(),
    )
    .await;
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(&destination);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(&destination);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(&destination);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir o instalador: {e}"),
            )
        })?;
    app.0.lock().await.push(
        format!(
            "Atualização {} baixada. Feche o ShadowPainel e conclua no instalador.",
            info.latest_version
        ),
        "success",
    );
    Ok(Json(json!({
        "ok": true,
        "version": info.latest_version,
        "path": destination,
        "message": "Instalador aberto. Feche o ShadowPainel para concluir a atualização.",
    })))
}

async fn rollback_update(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let state: Value = tokio::fs::read(update_state_path())
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "Ainda não há uma versão anterior registrada para voltar.",
            )
        })?;
    let previous = state["previous_version"]
        .as_str()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Versão anterior não encontrada."))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao preparar o download: {e}"),
            )
        })?;
    let release = fetch_release(&client, Some(&format!("v{previous}"))).await?;
    let (asset_name, asset_url, _size, digest) = release_asset(&release).ok_or_else(|| {
        err(
            StatusCode::NOT_FOUND,
            "A versão anterior não tem instalador para este sistema.",
        )
    })?;
    let dir = updates_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar atualizações: {e}"),
        )
    })?;
    let destination = dir.join(
        safe_filename(&asset_name)
            .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Nome de instalador inválido."))?,
    );
    save_download(&client, &asset_url, &destination).await?;
    if let Some(expected) = digest {
        let bytes = tokio::fs::read(&destination).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao verificar o instalador anterior: {e}"),
            )
        })?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&expected) {
            let _ = tokio::fs::remove_file(&destination).await;
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "O instalador anterior chegou corrompido e foi apagado. Tente novamente.",
            ));
        }
    }
    make_executable(&destination).await;
    let _ = state;
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(&destination);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(&destination);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(&destination);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir o instalador anterior: {e}"),
            )
        })?;
    app.0.lock().await.push(
        format!("Instalador da versão {previous} aberto para reversão."),
        "info",
    );
    Ok(Json(json!({
        "ok": true,
        "version": previous,
        "message": format!("Instalador da versão {previous} aberto."),
    })))
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

    if i.status != Status::Offline || i.installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor e aguarde instalações antes de excluí-lo.",
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
        let looks_like_server = detect_launch(&dir).is_some()
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

async fn save_download(client: &reqwest::Client, url: &str, path: &Path) -> Result<(), ApiErr> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao criar pasta: {e}"),
            )
        })?;
    }
    let mut response = client
        .get(url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Falha no download: {e}")))?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Download rejeitado: {e}")))?;
    let mut file = tokio::fs::File::create(path).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar {}: {e}", path.display()),
        )
    })?;
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        err(
            StatusCode::BAD_GATEWAY,
            format!("Falha ao ler download: {e}"),
        )
    })? {
        file.write_all(&chunk).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao salvar {}: {e}", path.display()),
            )
        })?;
    }
    file.flush().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao finalizar {}: {e}", path.display()),
        )
    })
}

async fn fabric_loader_version(client: &reqwest::Client, mc: &str) -> Result<String, ApiErr> {
    let loaders: Vec<Value> = client
        .get(format!("https://meta.fabricmc.net/v2/versions/loader/{mc}"))
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Fabric: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Fabric retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Fabric JSON: {e}")))?;
    loaders
        .first()
        .and_then(|v| v["loader"]["version"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                format!("Fabric não possui loader para Minecraft {mc}."),
            )
        })
}

async fn install_fabric(
    client: &reqwest::Client,
    dir: &Path,
    mc: &str,
    loader: Option<&str>,
) -> Result<(), ApiErr> {
    let loader = match loader.filter(|v| !v.trim().is_empty()) {
        Some(value) => value.to_string(),
        None => fabric_loader_version(client, mc).await?,
    };
    let installers: Vec<Value> = client
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Instalador Fabric: {e}")))?
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
                format!("Instalador Fabric JSON: {e}"),
            )
        })?;
    let installer_version = installers
        .first()
        .and_then(|v| v["version"].as_str())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Instalador Fabric não encontrado."))?;
    let url = format!(
        "https://meta.fabricmc.net/v2/versions/loader/{mc}/{loader}/{installer_version}/server/jar"
    );
    save_download(client, &url, &dir.join("fabric-server-launch.jar")).await
}

async fn run_installer(
    java: &str,
    dir: &Path,
    installer: &Path,
    args: &[&str],
) -> Result<(), ApiErr> {
    let mut command = Command::new(java);
    command
        .current_dir(dir)
        .arg("-jar")
        .arg(installer)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PACK_INSTALL_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            err(
                StatusCode::GATEWAY_TIMEOUT,
                "O instalador do modloader demorou demais e foi interrompido.",
            )
        })?
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao executar o instalador: {e}"),
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    let log: String = log
        .chars()
        .rev()
        .take(1600)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Err(err(
        StatusCode::BAD_GATEWAY,
        format!("O instalador falhou. Saída:\n{log}"),
    ))
}

async fn install_forge(
    client: &reqwest::Client,
    java: &str,
    dir: &Path,
    mc: &str,
    loader: &str,
) -> Result<(), ApiErr> {
    let url = format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{loader}/forge-{mc}-{loader}-installer.jar"
    );
    let installer = dir.join(".shadowpainel-forge-installer.jar");
    save_download(client, &url, &installer).await?;
    let result = run_installer(java, dir, &installer, &["--installServer"]).await;
    let _ = tokio::fs::remove_file(&installer).await;
    result
}

async fn install_neoforge(
    client: &reqwest::Client,
    java: &str,
    dir: &Path,
    loader: &str,
) -> Result<(), ApiErr> {
    let url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader}/neoforge-{loader}-installer.jar"
    );
    let installer = dir.join(".shadowpainel-neoforge-installer.jar");
    save_download(client, &url, &installer).await?;
    let result = run_installer(java, dir, &installer, &["--installServer"]).await;
    let _ = tokio::fs::remove_file(&installer).await;
    result
}

async fn install_quilt(
    client: &reqwest::Client,
    java: &str,
    dir: &Path,
    mc: &str,
    loader: Option<&str>,
) -> Result<(), ApiErr> {
    let installers: Vec<Value> = client
        .get("https://meta.quiltmc.org/v3/versions/installer")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Instalador Quilt: {e}")))?
        .error_for_status()
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Quilt retornou erro: {e}")))?
        .json()
        .await
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Instalador Quilt JSON: {e}"),
            )
        })?;
    let url = installers
        .first()
        .and_then(|v| v["url"].as_str())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Instalador Quilt não encontrado."))?;
    let installer = dir.join(".shadowpainel-quilt-installer.jar");
    save_download(client, url, &installer).await?;
    let installdir = format!("--install-dir={}", dir.display());
    let mut args = vec!["install", "server", mc, "--download-server", &installdir];
    if let Some(loader) = loader.filter(|v| !v.trim().is_empty()) {
        args.insert(3, loader);
    }
    let result = run_installer(java, dir, &installer, &args).await;
    let _ = tokio::fs::remove_file(&installer).await;
    result?;
    if detect_launch(dir).is_none() {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            "O instalador do Quilt terminou sem gerar um servidor inicializável.",
        ));
    }
    Ok(())
}

async fn install_vanilla(client: &reqwest::Client, dir: &Path, mc: &str) -> Result<(), ApiErr> {
    let manifest: Value = client
        .get("https://launchermeta.mojang.com/mc/game/version_manifest.json")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Mojang: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Mojang retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Manifesto Mojang: {e}")))?;
    let metadata_url = manifest["versions"]
        .as_array()
        .and_then(|list| list.iter().find(|v| v["id"] == mc))
        .and_then(|v| v["url"].as_str())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Versão Minecraft não encontrada."))?;
    let metadata: Value = client
        .get(metadata_url)
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Mojang: {e}")))?
        .error_for_status()
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Mojang retornou erro: {e}"),
            )
        })?
        .json()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Metadados Mojang: {e}")))?;
    let url = metadata["downloads"]["server"]["url"]
        .as_str()
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "Esta versão não oferece server.jar oficial.",
            )
        })?;
    save_download(client, url, &dir.join("server.jar")).await
}

async fn install_paper(
    client: &reqwest::Client,
    dir: &Path,
    mc: &str,
    kind: &str,
) -> Result<(), ApiErr> {
    let url = if kind == "paper" {
        let build: Value = client
            .get(format!(
                "https://fill.papermc.io/v3/projects/paper/versions/{mc}/builds/latest"
            ))
            .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
            .send()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Paper: {e}")))?
            .error_for_status()
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Paper retornou erro: {e}")))?
            .json()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Paper JSON: {e}")))?;
        build["downloads"]["server:default"]["url"]
            .as_str()
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Build Paper não encontrada."))?
            .to_string()
    } else {
        let builds: Value = client
            .get(format!("https://api.purpurmc.org/v2/purpur/{mc}"))
            .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
            .send()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Purpur: {e}")))?
            .error_for_status()
            .map_err(|e| {
                err(
                    StatusCode::BAD_GATEWAY,
                    format!("Purpur retornou erro: {e}"),
                )
            })?
            .json()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Purpur JSON: {e}")))?;
        let build = builds["builds"]["latest"]
            .as_str()
            .or_else(|| {
                builds["builds"]["all"]
                    .as_array()
                    .and_then(|b| b.last())
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "Build Purpur não encontrada."))?;
        format!("https://api.purpurmc.org/v2/purpur/{mc}/{build}/download")
    };
    save_download(client, &url, &dir.join("server.jar")).await
}

async fn install_loader(
    client: &reqwest::Client,
    java: &str,
    dir: &Path,
    mc: &str,
    loader: &str,
    loader_version: Option<&str>,
) -> Result<(), ApiErr> {
    match loader {
        "fabric" => install_fabric(client, dir, mc, loader_version).await,
        "quilt" => install_quilt(client, java, dir, mc, loader_version).await,
        "forge" => {
            let version = loader_version.unwrap_or_default().trim();
            if version.is_empty() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "A versão do Forge não foi informada.",
                ));
            }
            install_forge(client, java, dir, mc, version).await
        }
        "neoforge" => {
            let version = loader_version.unwrap_or_default().trim();
            if version.is_empty() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "A versão do NeoForge não foi informada.",
                ));
            }
            install_neoforge(client, java, dir, version).await
        }
        "paper" | "purpur" => install_paper(client, dir, mc, loader).await,
        _ => install_vanilla(client, dir, mc).await,
    }
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
            "Tipo de servidor não suportado.",
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
    if detect_launch(&dir).is_some() || dir.join("server.properties").is_file() {
        return Err(err(
            StatusCode::CONFLICT,
            "Esta pasta já possui uma instalação de servidor.",
        ));
    }

    {
        let i = app.0.lock().await;
        if i.status != Status::Offline || i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor e aguarde instalações antes de criar outro.",
            ));
        }
    }
    let java_path = ensure_java_for_version(&app, req.version.trim()).await?;
    {
        let mut i = app.0.lock().await;
        if i.status != Status::Offline || i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor e aguarde instalações antes de criar outro.",
            ));
        }
        i.installing = true;
    }

    let client = reqwest::Client::new();
    let result = install_loader(
        &client,
        &java_path,
        &dir,
        req.version.trim(),
        req.loader.trim(),
        None,
    )
    .await;

    let mut i = app.0.lock().await;
    i.installing = false;
    result.inspect_err(|_| {
        i.push(
            "Falha ao instalar o servidor. Veja os detalhes no aviso.",
            "error",
        );
    })?;
    i.cfg.server_name = req.server_name.trim().into();
    i.cfg.version = req.version.trim().into();
    i.cfg.loader = req.loader.trim().into();
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
    source: Option<String>,
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
    #[serde(default = "default_content_source")]
    source: String,
}

fn default_content_source() -> String {
    "modrinth".into()
}

fn content_source(source: Option<&str>) -> Result<&'static str, ApiErr> {
    match source.unwrap_or("modrinth").trim().to_lowercase().as_str() {
        "modrinth" | "" => Ok("modrinth"),
        "curseforge" | "curse" | "cf" => Ok("curseforge"),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "Fonte de conteúdo inválida (use modrinth ou curseforge).",
        )),
    }
}

fn valid_project_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn value_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
        .or_else(|| value.as_u64().map(|id| id.to_string()))
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
        .filter(|item| {
            item.source == "modrinth" && (item.title.is_none() || item.icon_url.is_none())
        })
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
        .filter(|item| item.source == "modrinth" && item.version_number.is_none())
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
                source: stored.map(|item| item.source.clone()),
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
    let (base, status) = {
        let i = app.0.lock().await;
        (expand(&i.cfg.server_dir), i.status)
    };
    if status != Status::Offline {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor antes de remover conteúdo.",
        ));
    }
    let dir = content_dir(&base, &req.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Tipo de conteúdo inválido."))?;

    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    let token = new_trash_token();
    let trash_dir = trash_root(&base)
        .join("content")
        .join(&token)
        .join(&req.kind);
    tokio::fs::create_dir_all(&trash_dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a lixeira: {e}"),
        )
    })?;

    for filename in req.filenames {
        let safe_name = std::path::Path::new(&filename)
            .file_name()
            .and_then(|x| x.to_str())
            .filter(|x| *x == filename && !x.is_empty());

        if let Some(name) = safe_name {
            let path = dir.join(name);
            if path.exists() {
                if let Err(e) = tokio::fs::rename(&path, trash_dir.join(name)).await {
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

    if deleted.is_empty() {
        let _ = tokio::fs::remove_dir_all(trash_root(&base).join("content").join(&token)).await;
        return Ok(Json(
            json!({"ok": false, "deleted": deleted, "errors": errors}),
        ));
    }

    let mut installed = read_installed_content(&base).await;
    let removed: Vec<InstalledContent> = installed
        .iter()
        .filter(|item| item.kind == req.kind && deleted.contains(&item.filename))
        .cloned()
        .collect();
    installed.retain(|x| {
        !deleted
            .iter()
            .any(|name| x.kind == req.kind && x.filename == *name)
    });
    write_installed_content(&base, &installed).await?;
    let removed_bytes = serde_json::to_vec_pretty(&removed).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a lixeira: {e}"),
        )
    })?;
    tokio::fs::write(
        trash_root(&base)
            .join("content")
            .join(&token)
            .join("removed.json"),
        removed_bytes,
    )
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a lixeira: {e}"),
        )
    })?;
    Ok(Json(
        json!({"ok": true, "deleted": deleted, "errors": errors, "undo_token": token}),
    ))
}

#[derive(Deserialize)]
struct ContentSearchRequest {
    query: String,
    kind: String,
    loader: String,
    version: String,
    category: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_content_limit")]
    limit: usize,
}

fn default_content_limit() -> usize {
    12
}

fn cf_search_hit(item: &Value, installed: bool) -> Value {
    let id = item.get("id").and_then(value_id).unwrap_or_default();
    let author = item
        .get("authors")
        .and_then(Value::as_array)
        .and_then(|authors| authors.first())
        .and_then(|author| author.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let icon = item
        .get("logo")
        .and_then(|logo| logo.get("thumbnailUrl").or_else(|| logo.get("url")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let latest = item
        .get("latestFilesIndexes")
        .and_then(Value::as_array)
        .and_then(|files| files.first());
    let version_number = latest
        .and_then(|file| file.get("filename"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let game_versions: Vec<String> = item
        .get("latestFilesIndexes")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|file| file.get("gameVersion").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    json!({
        "project_id": id,
        "title": item.get("name").and_then(Value::as_str).unwrap_or("Projeto CurseForge"),
        "description": item.get("summary").and_then(Value::as_str).unwrap_or_default(),
        "icon_url": icon,
        "downloads": item.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
        "author": author,
        "version_number": version_number,
        "game_versions": game_versions,
        "url": item.get("links").and_then(|links| links.get("websiteUrl")).and_then(Value::as_str),
        "installed": installed,
        "source": "curseforge",
    })
}

fn cf_version_item(file: &Value, loader: &str) -> Value {
    let game_versions: Vec<&str> = file
        .get("gameVersions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(Value::as_str)
                .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": file.get("id").and_then(value_id).unwrap_or_default(),
        "version_number": file.get("displayName").and_then(Value::as_str)
            .or_else(|| file.get("fileName").and_then(Value::as_str))
            .unwrap_or("Versão"),
        "filename": file.get("fileName").and_then(Value::as_str),
        "game_versions": game_versions,
        "loaders": [loader],
        "date_published": file.get("fileDate").and_then(Value::as_str),
        "downloads": file.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
        "source": "curseforge",
    })
}

async fn cf_file_metadata(
    cf: &CurseForge,
    project_id: &str,
    file_id: &str,
) -> Result<Value, ApiErr> {
    let data = cf
        .get(
            &format!("/mods/{project_id}/files/{file_id}"),
            &[("gameId", CF_GAME_ID.to_string())],
        )
        .await?;
    data.get("data").cloned().ok_or_else(|| {
        err(
            StatusCode::BAD_GATEWAY,
            "CurseForge não retornou o arquivo.",
        )
    })
}

async fn cf_file_download_url(
    cf: &CurseForge,
    project_id: &str,
    file_id: &str,
) -> Result<String, ApiErr> {
    let data = cf
        .get(
            &format!("/mods/{project_id}/files/{file_id}/download-url"),
            &[("gameId", CF_GAME_ID.to_string())],
        )
        .await?;
    data.get("data")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|url| url.starts_with("http"))
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "O autor desativou o download deste arquivo no CurseForge.",
            )
        })
}

async fn search_curseforge_content(
    app: &AppState,
    req: &ContentSearchRequest,
) -> Result<Json<Value>, ApiErr> {
    let class_id = content_class_id(&req.kind)?;
    let (base, key) = {
        let i = app.0.lock().await;
        (expand(&i.cfg.server_dir), i.cfg.curseforge_api_key.clone())
    };
    let client = reqwest::Client::new();
    let cf = CurseForge::new(&client, &key);
    let limit = req.limit.clamp(1, 24);
    let mut query: Vec<(&str, String)> = vec![
        ("gameId", CF_GAME_ID.to_string()),
        ("classId", class_id.to_string()),
        ("index", req.offset.to_string()),
        ("pageSize", limit.to_string()),
        ("sortField", "2".into()),
        ("sortOrder", "desc".into()),
    ];
    if !req.query.trim().is_empty() {
        query.push(("searchFilter", req.query.trim().to_string()));
    }
    if !req.version.trim().is_empty() {
        query.push(("gameVersion", req.version.trim().to_string()));
    }
    if req.kind == "mods" {
        let loader = content_loader(&req.kind, &req.loader);
        if let Some(loader_type) = cf_loader_type(&loader) {
            query.push(("modLoaderType", loader_type.to_string()));
        }
    }
    if let Some(category) = req
        .category
        .as_deref()
        .filter(|category| *category != "all")
        .and_then(|category| category.parse::<u32>().ok())
    {
        query.push(("categoryId", category.to_string()));
    }

    let data = cf.get("/mods/search", &query).await?;
    let installed: std::collections::HashSet<String> = read_installed_content(&base)
        .await
        .into_iter()
        .filter(|item| item.kind == req.kind && item.source == "curseforge")
        .map(|item| item.project_id)
        .collect();
    let hits: Vec<Value> = data["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let id = item.get("id").and_then(value_id).unwrap_or_default();
                    cf_search_hit(item, installed.contains(&id))
                })
                .collect()
        })
        .unwrap_or_default();
    let total = data
        .get("pagination")
        .and_then(|pagination| pagination.get("totalCount"))
        .cloned()
        .unwrap_or(json!(0));
    Ok(Json(json!({
        "hits": hits,
        "source": "curseforge",
        "total_hits": total,
        "offset": req.offset,
        "limit": limit,
    })))
}

async fn search_content(
    State(app): State<AppState>,
    Json(req): Json<ContentSearchRequest>,
) -> Result<Json<Value>, ApiErr> {
    let source = content_source(req.source.as_deref())?;
    if source == "curseforge" {
        return search_curseforge_content(&app, &req).await;
    }

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
    let version_facet = if req.version.trim().is_empty() {
        String::new()
    } else {
        format!(r#",["versions:{}"]"#, req.version)
    };

    let facets = format!(
        r#"[["project_type:{}"]{}{}{}]"#,
        project_type, version_facet, loader_facet, category_facet
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
        .filter(|x| x.kind == req.kind && x.source == "modrinth")
        .map(|x| x.project_id)
        .collect();
    data["source"] = Value::String("modrinth".into());
    if let Some(hits) = data.get_mut("hits").and_then(Value::as_array_mut) {
        for hit in hits {
            let id = hit
                .get("project_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            hit["installed"] = Value::Bool(installed.contains(id));
            hit["source"] = Value::String("modrinth".into());
        }
    }
    Ok(Json(data))
}

async fn modrinth_versions(
    client: &reqwest::Client,
    project_id: &str,
    loader: &str,
    version: &str,
) -> Result<Vec<Value>, ApiErr> {
    let mut request = client
        .get(format!(
            "https://api.modrinth.com/v2/project/{}/version",
            urlencoding::encode(project_id)
        ))
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"));
    if !loader.trim().is_empty() {
        request = request.query(&[("loaders", json!([loader]).to_string())]);
    }
    if !version.trim().is_empty() {
        request = request.query(&[("game_versions", json!([version]).to_string())]);
    }
    let data: Value = request
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
    let mut versions = match data {
        Value::Array(items) => items,
        item => vec![item],
    };
    for item in &mut versions {
        item["source"] = Value::String("modrinth".into());
    }
    Ok(versions)
}

#[derive(Deserialize)]
struct ContentVersionsQuery {
    query: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    loader: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    source: Option<String>,
}

async fn get_content_versions(
    State(app): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ContentVersionsQuery>,
) -> Result<Json<Value>, ApiErr> {
    let source = content_source(query.source.as_deref())?;
    if !valid_project_id(&query.query) {
        return Err(err(StatusCode::BAD_REQUEST, "Projeto inválido."));
    }
    if source == "curseforge" {
        if query.kind == "datapacks" {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "O CurseForge não possui catálogo de datapacks.",
            ));
        }
        let key = app.0.lock().await.cfg.curseforge_api_key.clone();
        let client = reqwest::Client::new();
        let cf = CurseForge::new(&client, &key);
        let loader = content_loader(&query.kind, &query.loader);
        let mut params: Vec<(&str, String)> = vec![("pageSize", "50".into())];
        if !query.version.trim().is_empty() {
            params.push(("gameVersion", query.version.trim().to_string()));
        }
        if let Some(loader_type) = cf_loader_type(&loader) {
            params.push(("modLoaderType", loader_type.to_string()));
        }
        let data = cf
            .get(&format!("/mods/{}/files", query.query), &params)
            .await?;
        let items: Vec<Value> = data["data"]
            .as_array()
            .map(|files| {
                files
                    .iter()
                    .map(|file| cf_version_item(file, &loader))
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Json(Value::Array(items)));
    }

    let loader = content_loader(&query.kind, &query.loader);
    let versions = modrinth_versions(
        &reqwest::Client::new(),
        &query.query,
        &loader,
        &query.version,
    )
    .await?;
    Ok(Json(Value::Array(versions)))
}

#[derive(Deserialize)]
struct ContentInstallRequest {
    project_id: String,
    kind: String,
    loader: String,
    version: String,
    version_id: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

async fn remember_content(base: &Path, record: InstalledContent) -> Result<(), ApiErr> {
    let mut installed = read_installed_content(base).await;
    installed.retain(|item| {
        !(item.kind == record.kind
            && item.filename == record.filename
            && item.source == record.source)
    });
    installed.push(record);
    write_installed_content(base, &installed).await?;
    Ok(())
}

async fn install_modrinth_content(
    client: &reqwest::Client,
    dir: &Path,
    req: &ContentInstallRequest,
) -> Result<InstalledContent, ApiErr> {
    let version = if let Some(version_id) = req.version_id.as_deref().filter(|id| !id.is_empty()) {
        let data: Value = client
            .get(format!(
                "https://api.modrinth.com/v2/version/{}",
                urlencoding::encode(version_id)
            ))
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
                    format!("Modrinth retornou erro: {e}"),
                )
            })?
            .json()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Resposta inválida: {e}")))?;
        Some(data)
    } else {
        modrinth_versions(
            client,
            &req.project_id,
            &content_loader(&req.kind, &req.loader),
            &req.version,
        )
        .await?
        .into_iter()
        .next()
    };
    let version = version.ok_or_else(|| {
        err(
            StatusCode::NOT_FOUND,
            "Nenhuma versão compatível encontrada.",
        )
    })?;

    let file = version["files"]
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .find(|file| file["primary"].as_bool().unwrap_or(false))
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
    let safe = safe_filename(filename)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Nome de arquivo inválido."))?;
    let download = file["url"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "URL de download ausente."))?;
    save_download(client, download, &dir.join(safe)).await?;

    Ok(InstalledContent {
        project_id: req.project_id.clone(),
        kind: req.kind.clone(),
        filename: safe.to_string(),
        version_id: version["id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| req.version_id.clone())
            .unwrap_or_default(),
        title: None,
        version_number: version["version_number"].as_str().map(str::to_owned),
        icon_url: None,
        source: "modrinth".into(),
    })
}

async fn install_curseforge_content(
    cf: &CurseForge,
    client: &reqwest::Client,
    dir: &Path,
    req: &ContentInstallRequest,
) -> Result<InstalledContent, ApiErr> {
    if req.kind == "datapacks" {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "O CurseForge não possui catálogo de datapacks.",
        ));
    }
    let loader = content_loader(&req.kind, &req.loader);
    let file = if let Some(file_id) = req.version_id.as_deref().filter(|id| !id.is_empty()) {
        cf_file_metadata(cf, &req.project_id, file_id).await?
    } else {
        let mut params: Vec<(&str, String)> = vec![("pageSize", "50".into())];
        if !req.version.trim().is_empty() {
            params.push(("gameVersion", req.version.trim().to_string()));
        }
        if let Some(loader_type) = cf_loader_type(&loader) {
            params.push(("modLoaderType", loader_type.to_string()));
        }
        let data = cf
            .get(&format!("/mods/{}/files", req.project_id), &params)
            .await?;
        data["data"]
            .as_array()
            .and_then(|files| files.first())
            .cloned()
            .ok_or_else(|| {
                err(
                    StatusCode::NOT_FOUND,
                    "Nenhum arquivo compatível encontrado.",
                )
            })?
    };
    let file_id = file.get("id").and_then(value_id).ok_or_else(|| {
        err(
            StatusCode::BAD_GATEWAY,
            "CurseForge não retornou o arquivo.",
        )
    })?;
    let filename = file["fileName"]
        .as_str()
        .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Nome de arquivo ausente."))?;
    let safe = safe_filename(filename)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Nome de arquivo inválido."))?;
    let download = match cf_file_download_url(cf, &req.project_id, &file_id).await {
        Ok(url) => url,
        Err(error) => file["downloadUrl"]
            .as_str()
            .map(str::to_owned)
            .ok_or(error)?,
    };
    save_download(client, &download, &dir.join(safe)).await?;

    let project = cf
        .get(&format!("/mods/{}", req.project_id), &[])
        .await
        .ok()
        .and_then(|data| data.get("data").cloned());
    Ok(InstalledContent {
        project_id: req.project_id.clone(),
        kind: req.kind.clone(),
        filename: safe.to_string(),
        version_id: file_id,
        title: project
            .as_ref()
            .and_then(|project| project.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        version_number: file["displayName"].as_str().map(str::to_owned),
        icon_url: project
            .as_ref()
            .and_then(|project| project.get("logo"))
            .and_then(|logo| logo.get("thumbnailUrl").or_else(|| logo.get("url")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        source: "curseforge".into(),
    })
}

async fn install_content(
    State(app): State<AppState>,
    Json(req): Json<ContentInstallRequest>,
) -> Result<Json<Value>, ApiErr> {
    let source = content_source(req.source.as_deref())?;
    if !valid_project_id(&req.project_id) {
        return Err(err(StatusCode::BAD_REQUEST, "Projeto inválido."));
    }
    let (base, key) = {
        let i = app.0.lock().await;
        if i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Aguarde a instalação em andamento antes de adicionar conteúdo.",
            ));
        }
        if i.status != Status::Offline {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor antes de instalar conteúdo. Isso evita arquivos corrompidos.",
            ));
        }
        let base = expand(&i.cfg.server_dir);
        (base, i.cfg.curseforge_api_key.clone())
    };
    let dir = content_dir(&base, &req.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Tipo de conteúdo inválido."))?;
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha na pasta: {e}"),
        )
    })?;
    automatic_backup(&app, &format!("antes-de-{}", req.kind)).await?;
    let client = reqwest::Client::new();
    let mut record = if source == "curseforge" {
        let cf = CurseForge::new(&client, &key);
        install_curseforge_content(&cf, &client, &dir, &req).await?
    } else {
        install_modrinth_content(&client, &dir, &req).await?
    };
    if record.source == "modrinth" {
        enrich_installed_content(&client, std::slice::from_mut(&mut record)).await;
    }
    let filename = record.filename.clone();
    remember_content(&base, record).await?;
    Ok(Json(json!({"ok": true, "filename": filename})))
}

// ───────────────────────── CurseForge ─────────────────────────

async fn curseforge_status(State(app): State<AppState>) -> Json<Value> {
    let key = app.0.lock().await.cfg.curseforge_api_key.clone();
    let client = reqwest::Client::new();
    let cf = CurseForge::new(&client, &key);
    let probe = cf
        .get(
            "/mods/search",
            &[
                ("gameId", CF_GAME_ID.to_string()),
                ("classId", CF_CLASS_MODS.to_string()),
                ("pageSize", "1".into()),
            ],
        )
        .await;
    let (ok, message) = match probe {
        Ok(_) => (
            true,
            if cf.mode() == "oficial" {
                "Chave oficial validada. O catálogo usa a API do CurseForge.".to_string()
            } else {
                "Usando o proxy público api.curse.tools (sem chave oficial).".to_string()
            },
        ),
        Err((_, Json(value))) => (
            false,
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("CurseForge indisponível no momento.")
                .to_string(),
        ),
    };
    Json(json!({
        "mode": cf.mode(),
        "configured": cf.key.is_some(),
        "ok": ok,
        "message": message,
    }))
}

#[derive(Deserialize)]
struct CurseForgeConfigRequest {
    api_key: String,
}

async fn save_curseforge_config(
    State(app): State<AppState>,
    Json(req): Json<CurseForgeConfigRequest>,
) -> ApiResult {
    let key = req.api_key.trim().to_string();
    if key.chars().any(char::is_control) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "A chave do CurseForge contém caracteres inválidos.",
        ));
    }
    if !key.is_empty() {
        let client = reqwest::Client::new();
        let cf = CurseForge::new(&client, &key);
        if let Err((_, Json(value))) = cf
            .get(
                "/mods/search",
                &[
                    ("gameId", CF_GAME_ID.to_string()),
                    ("classId", CF_CLASS_MODS.to_string()),
                    ("pageSize", "1".into()),
                ],
            )
            .await
        {
            let detail = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("chave recusada");
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("Não foi possível validar a chave: {detail}"),
            ));
        }
    }
    let mut i = app.0.lock().await;
    i.cfg.curseforge_api_key = key.clone();
    i.persist().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar a chave: {e}"),
        )
    })?;
    i.push(
        if key.is_empty() {
            "CurseForge: usando o proxy público."
        } else {
            "CurseForge: chave oficial validada e salva."
        },
        "success",
    );
    Ok(Json(view(&mut i)))
}

// ───────────────────────── Modpacks ─────────────────────────

#[derive(Deserialize)]
struct ModpackSearchRequest {
    #[serde(default)]
    query: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    loader: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_content_limit")]
    limit: usize,
}

async fn search_modpacks(
    State(app): State<AppState>,
    Json(req): Json<ModpackSearchRequest>,
) -> Result<Json<Value>, ApiErr> {
    let source = content_source(req.source.as_deref())?;
    let limit = req.limit.clamp(1, 24);
    let client = reqwest::Client::new();
    if source == "curseforge" {
        let key = app.0.lock().await.cfg.curseforge_api_key.clone();
        let cf = CurseForge::new(&client, &key);
        let mut query: Vec<(&str, String)> = vec![
            ("gameId", CF_GAME_ID.to_string()),
            ("classId", CF_CLASS_MODPACKS.to_string()),
            ("index", req.offset.to_string()),
            ("pageSize", limit.to_string()),
            ("sortField", "2".into()),
            ("sortOrder", "desc".into()),
        ];
        if !req.query.trim().is_empty() {
            query.push(("searchFilter", req.query.trim().to_string()));
        }
        if !req.version.trim().is_empty() {
            query.push(("gameVersion", req.version.trim().to_string()));
        }
        if let Some(loader_type) = cf_loader_type(&req.loader) {
            query.push(("modLoaderType", loader_type.to_string()));
        }
        let data = cf.get("/mods/search", &query).await?;
        let hits: Vec<Value> = data["data"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| cf_search_hit(item, false))
                    .collect()
            })
            .unwrap_or_default();
        let total = data
            .get("pagination")
            .and_then(|pagination| pagination.get("totalCount"))
            .cloned()
            .unwrap_or(json!(0));
        return Ok(Json(json!({
            "hits": hits,
            "source": "curseforge",
            "total_hits": total,
            "offset": req.offset,
            "limit": limit,
        })));
    }

    let mut facets = vec![json!(["project_type:modpack"])];
    if !req.version.trim().is_empty() {
        facets.push(json!(format!("versions:{}", req.version.trim())));
    }
    if !req.loader.trim().is_empty() && req.loader != "vanilla" {
        facets.push(json!(format!("categories:{}", req.loader.trim())));
    }
    let data: Value = client
        .get("https://api.modrinth.com/v2/search")
        .header("User-Agent", format!("ShadowPainel/{APP_VERSION}"))
        .query(&[
            ("query", req.query.trim().to_string()),
            (
                "facets",
                serde_json::to_string(&facets).unwrap_or_else(|_| "[]".into()),
            ),
            ("limit", limit.to_string()),
            ("offset", req.offset.to_string()),
        ])
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
    let mut data = data;
    data["source"] = Value::String("modrinth".into());
    if let Some(hits) = data.get_mut("hits").and_then(Value::as_array_mut) {
        for hit in hits {
            hit["source"] = Value::String("modrinth".into());
        }
    }
    Ok(Json(data))
}

#[derive(Deserialize)]
struct ModpackVersionsQuery {
    project_id: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    loader: String,
}

async fn modpack_versions(
    State(app): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ModpackVersionsQuery>,
) -> Result<Json<Value>, ApiErr> {
    let source = content_source(query.source.as_deref())?;
    if !valid_project_id(&query.project_id) {
        return Err(err(StatusCode::BAD_REQUEST, "Projeto inválido."));
    }
    let client = reqwest::Client::new();
    if source == "curseforge" {
        let key = app.0.lock().await.cfg.curseforge_api_key.clone();
        let cf = CurseForge::new(&client, &key);
        let mut params: Vec<(&str, String)> = vec![("pageSize", "50".into())];
        if !query.version.trim().is_empty() {
            params.push(("gameVersion", query.version.trim().to_string()));
        }
        if let Some(loader_type) = cf_loader_type(&query.loader) {
            params.push(("modLoaderType", loader_type.to_string()));
        }
        let data = cf
            .get(&format!("/mods/{}/files", query.project_id), &params)
            .await?;
        let loader = if query.loader.trim().is_empty() {
            "modpack"
        } else {
            query.loader.trim()
        };
        let items: Vec<Value> = data["data"]
            .as_array()
            .map(|files| {
                files
                    .iter()
                    .map(|file| cf_version_item(file, loader))
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Json(Value::Array(items)));
    }
    let loader = if query.loader == "vanilla" || query.loader.trim().is_empty() {
        ""
    } else {
        query.loader.trim()
    };
    let versions = modrinth_versions(&client, &query.project_id, loader, &query.version).await?;
    Ok(Json(Value::Array(versions)))
}

struct ModpackPlan {
    minecraft: String,
    loader: String,
    loader_version: Option<String>,
    mrpack_files: Vec<(String, String)>,
    curseforge_files: Vec<(String, String)>,
}

impl ModpackPlan {
    fn empty(minecraft: String, loader: String, loader_version: Option<String>) -> Self {
        Self {
            minecraft,
            loader,
            loader_version,
            mrpack_files: Vec::new(),
            curseforge_files: Vec::new(),
        }
    }
}

fn parse_mrpack(index: &Value) -> Result<ModpackPlan, ApiErr> {
    let dependencies = index.get("dependencies").ok_or_else(|| {
        err(
            StatusCode::BAD_GATEWAY,
            "O modpack não possui a lista de dependências.",
        )
    })?;
    let minecraft = dependencies
        .get("minecraft")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            err(
                StatusCode::BAD_GATEWAY,
                "O modpack não informa a versão do Minecraft.",
            )
        })?;
    let mut loader = "vanilla".to_string();
    let mut loader_version = None;
    for (key, name) in [
        ("fabric-loader", "fabric"),
        ("quilt-loader", "quilt"),
        ("forge", "forge"),
        ("neoforge", "neoforge"),
    ] {
        if let Some(version) = dependencies.get(key).and_then(Value::as_str) {
            loader = name.into();
            loader_version = Some(version.to_string());
            break;
        }
    }
    let mut plan = ModpackPlan::empty(minecraft.to_string(), loader, loader_version);
    for file in index
        .get("files")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if file
            .pointer("/env/server")
            .and_then(Value::as_str)
            .is_some_and(|env| env == "unsupported")
        {
            continue;
        }
        let path = file["path"].as_str().ok_or_else(|| {
            err(
                StatusCode::BAD_GATEWAY,
                "O modpack contém um arquivo sem caminho.",
            )
        })?;
        if safe_pack_path(Path::new("."), path).is_none() {
            continue;
        }
        let url = file["downloads"]
            .as_array()
            .and_then(|downloads| downloads.first())
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    StatusCode::BAD_GATEWAY,
                    format!("O arquivo {path} não possui download."),
                )
            })?;
        plan.mrpack_files.push((path.to_string(), url.to_string()));
    }
    Ok(plan)
}

fn parse_curseforge_manifest(manifest: &Value) -> Result<ModpackPlan, ApiErr> {
    let minecraft = manifest
        .pointer("/minecraft/version")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            err(
                StatusCode::BAD_GATEWAY,
                "O modpack não informa a versão do Minecraft.",
            )
        })?;
    let loaders = manifest
        .pointer("/minecraft/modLoaders")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            err(
                StatusCode::BAD_GATEWAY,
                "O modpack não informa o modloader.",
            )
        })?;
    let loader_id = loaders
        .iter()
        .find(|loader| loader["primary"].as_bool().unwrap_or(false))
        .or_else(|| loaders.first())
        .and_then(|loader| loader["id"].as_str())
        .ok_or_else(|| {
            err(
                StatusCode::BAD_GATEWAY,
                "O modpack não informa o modloader.",
            )
        })?;
    let (loader, loader_version) = split_loader_id(loader_id).ok_or_else(|| {
        err(
            StatusCode::BAD_GATEWAY,
            format!("Modloader desconhecido: {loader_id}"),
        )
    })?;
    let mut plan = ModpackPlan::empty(minecraft.to_string(), loader, Some(loader_version));
    for file in manifest
        .get("files")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let project_id = file
            .get("projectID")
            .and_then(value_id)
            .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Mod sem projectID."))?;
        let file_id = file
            .get("fileID")
            .and_then(value_id)
            .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Mod sem fileID."))?;
        plan.curseforge_files.push((project_id, file_id));
    }
    Ok(plan)
}

async fn read_zip_json(path: &Path, entry: &str) -> Result<Option<Value>, ApiErr> {
    let path = path.to_path_buf();
    let entry = entry.to_string();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut entry = match archive.by_name(&entry) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(Some(bytes))
    })
    .await
    .map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao ler o pacote: {e}"),
        )
    })?
    .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Pacote inválido: {e}")))?;
    match bytes {
        Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("JSON do pacote inválido: {e}"),
            )
        }),
        None => Ok(None),
    }
}

fn extract_zip_blocking(
    zip_path: PathBuf,
    destination: PathBuf,
    prefixes: Vec<String>,
) -> Result<usize, String> {
    let file = fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut extracted = 0;
    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let relative = if prefixes.is_empty() {
            Some(name.as_str())
        } else {
            prefixes.iter().find_map(|prefix| name.strip_prefix(prefix))
        };
        let Some(relative) = relative.map(|value| value.trim_start_matches('/')) else {
            continue;
        };
        if relative.is_empty()
            || relative.starts_with("backups/")
            || relative.starts_with(".shadowpainel")
        {
            continue;
        }
        let Some(path) = safe_pack_path(&destination, relative) else {
            continue;
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut output = fs::File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
        extracted += 1;
    }
    Ok(extracted)
}

async fn extract_zip(
    zip_path: &Path,
    destination: &Path,
    prefixes: Vec<String>,
) -> Result<usize, ApiErr> {
    let zip_path = zip_path.to_path_buf();
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || extract_zip_blocking(zip_path, destination, prefixes))
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao extrair o pacote: {e}"),
            )
        })?
        .map_err(|e| {
            err(
                StatusCode::BAD_GATEWAY,
                format!("Falha ao extrair o pacote: {e}"),
            )
        })
}

fn merge_tree_blocking(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            merge_tree_blocking(&from, &to)?;
            continue;
        }
        if to.exists() {
            if to.is_dir() {
                fs::remove_dir_all(&to).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&to).map_err(|e| e.to_string())?;
            }
        }
        if fs::rename(&from, &to).is_err() {
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
            let _ = fs::remove_file(&from);
        }
    }
    Ok(())
}

async fn merge_tree(source: &Path, destination: &Path) -> Result<(), ApiErr> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || merge_tree_blocking(&source, &destination))
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao mover arquivos do modpack: {e}"),
            )
        })?
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao mover arquivos do modpack: {e}"),
            )
        })
}

async fn install_log(app: &AppState, text: impl Into<String>) {
    let mut i = app.0.lock().await;
    i.push(text, "info");
}

fn api_error_text(error: ApiErr) -> String {
    let (_, Json(value)) = error;
    value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("erro desconhecido")
        .to_string()
}

async fn download_modrinth_files(
    client: &reqwest::Client,
    destination: &Path,
    files: Vec<(String, String)>,
) -> Result<usize, ApiErr> {
    if files.is_empty() {
        return Ok(0);
    }
    let semaphore = Arc::new(Semaphore::new(PACK_DOWNLOAD_CONCURRENCY));
    let mut tasks = JoinSet::new();
    let total = files.len();
    for (relative, url) in files {
        let client = client.clone();
        let destination = destination.to_path_buf();
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|_| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Fila de downloads interrompida.",
                )
            })?;
            let path = safe_pack_path(&destination, &relative).ok_or_else(|| {
                err(
                    StatusCode::BAD_GATEWAY,
                    format!("Caminho inválido no modpack: {relative}"),
                )
            })?;
            save_download(&client, &url, &path).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha em um download do modpack: {e}"),
            )
        })??;
    }
    Ok(total)
}

async fn download_curseforge_files(
    cf: &CurseForge,
    client: &reqwest::Client,
    destination: &Path,
    files: Vec<(String, String)>,
) -> Result<usize, ApiErr> {
    if files.is_empty() {
        return Ok(0);
    }
    let semaphore = Arc::new(Semaphore::new(PACK_DOWNLOAD_CONCURRENCY));
    let mut tasks = JoinSet::new();
    let total = files.len();
    for (project_id, file_id) in files {
        let cf = cf.clone();
        let client = client.clone();
        let destination = destination.join("mods");
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|_| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Fila de downloads interrompida.",
                )
            })?;
            let meta = cf_file_metadata(&cf, &project_id, &file_id).await?;
            let filename = meta["fileName"]
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Arquivo sem nome."))?;
            let safe = safe_filename(filename)
                .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Nome de arquivo inválido."))?;
            let url = match cf_file_download_url(&cf, &project_id, &file_id).await {
                Ok(url) => url,
                Err(error) => meta["downloadUrl"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(error)?,
            };
            save_download(&client, &url, &destination.join(safe)).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha em um download do modpack: {e}"),
            )
        })??;
    }
    Ok(total)
}

async fn cf_latest_modpack_file(
    cf: &CurseForge,
    project_id: &str,
    version_id: Option<&str>,
) -> Result<Value, ApiErr> {
    if let Some(version_id) = version_id.filter(|id| !id.is_empty()) {
        return cf_file_metadata(cf, project_id, version_id).await;
    }
    let data = cf
        .get(
            &format!("/mods/{project_id}/files"),
            &[("pageSize", "50".into())],
        )
        .await?;
    data["data"]
        .as_array()
        .and_then(|files| {
            files.iter().max_by(|left, right| {
                left.get("fileDate")
                    .and_then(Value::as_str)
                    .cmp(&right.get("fileDate").and_then(Value::as_str))
            })
        })
        .cloned()
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "O modpack não possui arquivos disponíveis.",
            )
        })
}

struct ModpackInstallRef<'a> {
    source: &'a str,
    project_id: &'a str,
    version_id: Option<&'a str>,
    curseforge_key: &'a str,
    accept_eula: bool,
}

async fn install_modpack_inner(
    app: &AppState,
    client: &reqwest::Client,
    base: &Path,
    install: ModpackInstallRef<'_>,
) -> Result<(String, String), ApiErr> {
    let ModpackInstallRef {
        source,
        project_id,
        version_id,
        curseforge_key,
        accept_eula,
    } = install;
    let staging = base.join(".shadowpainel-pack-staging");
    let pack_path = base.join(".shadowpainel-pack.zip");
    let server_pack_path = base.join(".shadowpainel-pack-server.zip");
    let _ = tokio::fs::remove_file(&pack_path).await;
    let _ = tokio::fs::remove_file(&server_pack_path).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a instalação: {e}"),
        )
    })?;

    let outcome = async {
        let (plan, cf, pack_file) = if source == "curseforge" {
            let cf = CurseForge::new(client, curseforge_key);
            let pack_file = cf_latest_modpack_file(&cf, project_id, version_id).await?;
            let pack_file_id = pack_file
                .get("id")
                .and_then(value_id)
                .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "Arquivo do modpack sem id."))?;
            install_log(app, "Baixando o modpack do CurseForge…").await;
            let pack_url = cf_file_download_url(&cf, project_id, &pack_file_id).await?;
            save_download(client, &pack_url, &pack_path).await?;
            let manifest = read_zip_json(&pack_path, "manifest.json")
                .await?
                .ok_or_else(|| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        "O modpack do CurseForge não contém manifest.json.",
                    )
                })?;
            let plan = parse_curseforge_manifest(&manifest)?;
            (plan, Some(cf), Some(pack_file))
        } else {
            install_log(app, "Baixando o modpack do Modrinth…").await;
            let versions = modrinth_versions(client, project_id, "", "").await?;
            let version = if let Some(version_id) = version_id.filter(|id| !id.is_empty()) {
                versions
                    .into_iter()
                    .find(|version| version["id"].as_str() == Some(version_id))
                    .ok_or_else(|| {
                        err(StatusCode::NOT_FOUND, "Versão do modpack não encontrada.")
                    })?
            } else {
                versions.into_iter().next().ok_or_else(|| {
                    err(
                        StatusCode::NOT_FOUND,
                        "O modpack não possui versões disponíveis.",
                    )
                })?
            };
            let file = version["files"]
                .as_array()
                .and_then(|files| {
                    files
                        .iter()
                        .find(|file| file["primary"].as_bool().unwrap_or(false))
                        .or_else(|| files.first())
                })
                .ok_or_else(|| {
                    err(
                        StatusCode::NOT_FOUND,
                        "O modpack não possui arquivo para download.",
                    )
                })?;
            let url = file["url"]
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_GATEWAY, "URL do modpack ausente."))?;
            save_download(client, url, &pack_path).await?;
            let index = read_zip_json(&pack_path, "modrinth.index.json")
                .await?
                .ok_or_else(|| {
                    err(
                        StatusCode::BAD_GATEWAY,
                        "O modpack não contém modrinth.index.json.",
                    )
                })?;
            let plan = parse_mrpack(&index)?;
            (plan, None, None)
        };

        install_log(
            app,
            format!("Instalando {} {}…", plan.loader, plan.minecraft),
        )
        .await;

        let mut server_pack_used = false;
        if let (Some(cf), Some(pack_file)) = (cf.as_ref(), pack_file.as_ref()) {
            if let Some(server_pack_id) = pack_file
                .get("serverPackFileId")
                .and_then(value_id)
                .filter(|id| id != "0")
            {
                install_log(app, "Baixando o server pack oficial…").await;
                match cf_file_download_url(cf, project_id, &server_pack_id).await {
                    Ok(url) => match save_download(client, &url, &server_pack_path).await {
                        Ok(()) => {
                            match extract_zip(&server_pack_path, &staging, Vec::new()).await {
                                Ok(count) => {
                                    server_pack_used = true;
                                    install_log(
                                        app,
                                        format!("Server pack extraído ({count} arquivos)."),
                                    )
                                    .await;
                                }
                                Err(error) => {
                                    install_log(
                                        app,
                                        format!(
                                            "Server pack inválido; usando o manifest. {}",
                                            api_error_text(error)
                                        ),
                                    )
                                    .await;
                                }
                            }
                        }
                        Err(error) => {
                            install_log(
                                app,
                                format!(
                                    "Falha ao baixar o server pack; usando o manifest. {}",
                                    api_error_text(error)
                                ),
                            )
                            .await;
                        }
                    },
                    Err(error) => {
                        install_log(
                            app,
                            format!(
                                "Server pack indisponível; usando o manifest. {}",
                                api_error_text(error)
                            ),
                        )
                        .await;
                    }
                }
                let _ = tokio::fs::remove_file(&server_pack_path).await;
            }
        }

        if !server_pack_used {
            install_log(app, "Extraindo overrides do modpack…").await;
            extract_zip(&pack_path, &staging, vec!["overrides/".into()]).await?;
            extract_zip(&pack_path, &staging, vec!["server-overrides/".into()]).await?;
            if let Some(cf) = cf.as_ref() {
                install_log(
                    app,
                    format!(
                        "Baixando {} mods do CurseForge…",
                        plan.curseforge_files.len()
                    ),
                )
                .await;
                download_curseforge_files(cf, client, &staging, plan.curseforge_files).await?;
            } else {
                install_log(
                    app,
                    format!("Baixando {} arquivos do modpack…", plan.mrpack_files.len()),
                )
                .await;
                download_modrinth_files(client, &staging, plan.mrpack_files).await?;
            }
        }

        let _ = tokio::fs::remove_file(&pack_path).await;
        let java_path = ensure_java_for_version(app, &plan.minecraft).await?;
        install_loader(
            client,
            &java_path,
            base,
            &plan.minecraft,
            &plan.loader,
            plan.loader_version.as_deref(),
        )
        .await?;
        merge_tree(&staging, base).await?;
        if accept_eula {
            tokio::fs::write(base.join("eula.txt"), "eula=true\n")
                .await
                .map_err(|e| {
                    err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Falha ao aceitar o EULA: {e}"),
                    )
                })?;
        }
        if detect_launch(base).is_none() {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                "A instalação terminou sem gerar um servidor inicializável.",
            ));
        }
        Ok((plan.minecraft, plan.loader))
    }
    .await;

    let _ = tokio::fs::remove_file(&pack_path).await;
    let _ = tokio::fs::remove_file(&server_pack_path).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    outcome
}

#[derive(Deserialize)]
struct ModpackInstallRequest {
    #[serde(default)]
    source: Option<String>,
    project_id: String,
    version_id: Option<String>,
    server_name: String,
    server_dir: String,
    memory_limit_gb: u64,
    #[serde(default)]
    accept_eula: bool,
}

async fn install_modpack(
    State(app): State<AppState>,
    Json(req): Json<ModpackInstallRequest>,
) -> ApiResult {
    let source = content_source(req.source.as_deref())?;
    if !valid_project_id(&req.project_id) {
        return Err(err(StatusCode::BAD_REQUEST, "Projeto inválido."));
    }
    if req.server_name.trim().is_empty() || req.server_dir.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Nome e pasta do servidor são obrigatórios.",
        ));
    }
    if !(1..=64).contains(&req.memory_limit_gb) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Memória deve ficar entre 1 e 64 GB.",
        ));
    }
    let base = expand(&req.server_dir);
    tokio::fs::create_dir_all(&base).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao criar a pasta do servidor: {e}"),
        )
    })?;
    if detect_launch(&base).is_some() || base.join("server.properties").is_file() {
        return Err(err(
            StatusCode::CONFLICT,
            "Esta pasta já possui uma instalação de servidor.",
        ));
    }
    let curseforge_key = {
        let mut i = app.0.lock().await;
        if i.status != Status::Offline || i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor e aguarde instalações antes de criar outro.",
            ));
        }
        i.installing = true;
        i.cfg.curseforge_api_key.clone()
    };

    let client = reqwest::Client::new();
    let result = install_modpack_inner(
        &app,
        &client,
        &base,
        ModpackInstallRef {
            source,
            project_id: &req.project_id,
            version_id: req.version_id.as_deref(),
            curseforge_key: &curseforge_key,
            accept_eula: req.accept_eula,
        },
    )
    .await;

    let mut i = app.0.lock().await;
    i.installing = false;
    match result {
        Ok((minecraft, loader)) => {
            i.cfg.server_name = req.server_name.trim().to_string();
            i.cfg.version = minecraft;
            i.cfg.loader = loader;
            i.cfg.server_dir = req.server_dir.trim().to_string();
            i.cfg.memory_limit_gb = req.memory_limit_gb;
            let server_dir = i.cfg.server_dir.clone();
            i.persist().await.map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Modpack instalado, mas falhou ao salvar a configuração: {e}"),
                )
            })?;
            let eula_note = if req.accept_eula {
                "EULA aceita."
            } else {
                "Aceite o EULA para iniciar."
            };
            i.push(
                format!("Modpack instalado em {server_dir}. {eula_note}"),
                "success",
            );
            Ok(Json(view(&mut i)))
        }
        Err(error) => {
            i.push(
                "Falha ao instalar o modpack. Os detalhes estão no erro exibido.",
                "error",
            );
            Err(error)
        }
    }
}

// ───────────────────────── Backups ─────────────────────────

#[derive(Serialize)]
struct BackupItem {
    name: String,
    size: u64,
    created: String,
    #[serde(skip)]
    modified: Option<std::time::SystemTime>,
}

fn backups_dir(base: &Path) -> PathBuf {
    base.join("backups")
}

async fn backup_items(base: &Path) -> Result<Vec<BackupItem>, ApiErr> {
    let dir = backups_dir(base);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a pasta de backups: {e}"),
        )
    })?;
    let mut items = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao listar backups: {e}"),
        )
    })?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao ler backups: {e}"),
        )
    })? {
        let metadata = match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".zip") {
            continue;
        }
        let modified = metadata.modified().ok();
        let created = modified
            .map(|time| {
                let datetime: DateTime<Local> = time.into();
                datetime.format("%d/%m/%Y %H:%M").to_string()
            })
            .unwrap_or_default();
        items.push(BackupItem {
            name,
            size: metadata.len(),
            created,
            modified,
        });
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.modified));
    Ok(items)
}

async fn list_backups(State(app): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    Ok(Json(json!({"items": backup_items(&base).await?})))
}

fn create_backup_blocking(base: &Path, destination: &Path) -> Result<u64, String> {
    let file = fs::File::create(destination).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    fn add_path(
        zip: &mut ZipWriter<fs::File>,
        path: &Path,
        relative: &Path,
        options: SimpleFileOptions,
        top_level: bool,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if top_level {
            let skip = matches!(
                name.as_str(),
                "backups" | "logs" | "cache" | "libraries" | "versions" | "crash-reports"
            ) || name.starts_with(".shadowpainel")
                || name.ends_with(".jar")
                || name.ends_with(".log")
                || name.ends_with(".log.gz");
            if skip {
                return Ok(());
            }
        }
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    add_path(
                        zip,
                        &entry.path(),
                        &relative.join(entry.file_name()),
                        options,
                        false,
                    )?;
                }
            }
            return Ok(());
        }
        if !metadata.is_file() {
            return Ok(());
        }
        let entry_name = relative.to_string_lossy().replace('\\', "/");
        zip.start_file(entry_name, options)
            .map_err(|e| e.to_string())?;
        let mut source = fs::File::open(path).map_err(|e| e.to_string())?;
        std::io::copy(&mut source, zip).map_err(|e| e.to_string())?;
        Ok(())
    }

    for entry in fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        add_path(
            &mut zip,
            &entry.path(),
            Path::new(&entry.file_name()),
            options,
            true,
        )?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    fs::metadata(destination)
        .map(|metadata| metadata.len())
        .map_err(|e| e.to_string())
}

async fn write_backup_file(base: &Path, destination: &Path) -> Result<u64, ApiErr> {
    let base_copy = base.to_path_buf();
    let destination_copy = destination.to_path_buf();
    tokio::task::spawn_blocking(move || create_backup_blocking(&base_copy, &destination_copy))
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao criar o backup: {e}"),
            )
        })?
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao criar o backup: {e}"),
            )
        })
}

async fn automatic_backup(app: &AppState, reason: &str) -> Result<Option<String>, ApiErr> {
    let (enabled, base, status, installing) = {
        let i = app.0.lock().await;
        (
            i.cfg.auto_backup,
            expand(&i.cfg.server_dir),
            i.status,
            i.installing,
        )
    };
    if !enabled || detect_launch(&base).is_none() {
        return Ok(None);
    }
    if status != Status::Offline || installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor para criar o backup automático antes de instalar.",
        ));
    }
    let dir = backups_dir(&base);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a pasta de backups: {e}"),
        )
    })?;
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("automatico-") || !name.ends_with(".zip") {
                continue;
            }
            if let Ok(metadata) = entry.metadata().await {
                if metadata
                    .modified()
                    .ok()
                    .and_then(|time| SystemTime::now().duration_since(time).ok())
                    .is_some_and(|age| age < Duration::from_secs(600))
                {
                    return Ok(None);
                }
            }
        }
    }
    let name = format!(
        "automatico-{}-{}.zip",
        safe_filename(reason).unwrap_or("antes-de-instalar"),
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let destination = dir.join(&name);
    write_backup_file(&base, &destination).await?;
    app.0.lock().await.push(
        format!("Backup automático criado antes de instalar: {name}"),
        "success",
    );
    Ok(Some(name))
}

#[derive(Deserialize)]
struct BackupCreateRequest {
    name: Option<String>,
}

async fn create_backup(
    State(app): State<AppState>,
    Json(req): Json<BackupCreateRequest>,
) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        if i.status != Status::Offline || i.installing {
            return Err(err(
                StatusCode::CONFLICT,
                "Pare o servidor e aguarde instalações para criar um backup.",
            ));
        }
        expand(&i.cfg.server_dir)
    };
    let requested = req.name.as_deref().unwrap_or("").trim();
    let name = if requested.is_empty() {
        format!("backup-{}.zip", Local::now().format("%Y%m%d-%H%M%S"))
    } else {
        let name = if requested.ends_with(".zip") {
            requested.to_string()
        } else {
            format!("{requested}.zip")
        };
        safe_filename(&name)
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Nome de backup inválido."))?
            .to_string()
    };
    let dir = backups_dir(&base);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a pasta de backups: {e}"),
        )
    })?;
    let destination = dir.join(&name);
    if destination.exists() {
        return Err(err(
            StatusCode::CONFLICT,
            "Já existe um backup com esse nome.",
        ));
    }
    let base_copy = base.clone();
    let destination_copy = destination.clone();
    let size =
        tokio::task::spawn_blocking(move || create_backup_blocking(&base_copy, &destination_copy))
            .await
            .map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Falha ao criar o backup: {e}"),
                )
            })?
            .map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Falha ao criar o backup: {e}"),
                )
            })?;
    let mut i = app.0.lock().await;
    i.push(format!("Backup criado: {name}"), "success");
    drop(i);
    Ok(Json(json!({
        "ok": true,
        "name": name,
        "size": size,
        "items": backup_items(&base).await?,
    })))
}

#[derive(Deserialize)]
struct BackupDeleteRequest {
    name: String,
}

async fn delete_backup(
    State(app): State<AppState>,
    Json(req): Json<BackupDeleteRequest>,
) -> Result<Json<Value>, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    let name = safe_filename(req.name.trim())
        .filter(|name| name.ends_with(".zip"))
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Backup inválido."))?;
    let path = backups_dir(&base).join(name);
    if !path.is_file() {
        return Err(err(StatusCode::NOT_FOUND, "Backup não encontrado."));
    }
    let token = new_trash_token();
    let trash = trash_root(&base).join("backups").join(&token);
    tokio::fs::create_dir_all(&trash).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao preparar a lixeira: {e}"),
        )
    })?;
    tokio::fs::rename(&path, trash.join(name))
        .await
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao excluir o backup: {e}"),
            )
        })?;
    Ok(Json(json!({"ok": true, "undo_token": token})))
}

#[derive(Deserialize)]
struct TrashRestoreRequest {
    token: String,
}

async fn restore_trash(
    State(app): State<AppState>,
    Json(req): Json<TrashRestoreRequest>,
) -> Result<Json<Value>, ApiErr> {
    if !valid_trash_token(req.token.trim()) {
        return Err(err(StatusCode::BAD_REQUEST, "Ação inválida."));
    }
    let (base, status) = {
        let i = app.0.lock().await;
        (expand(&i.cfg.server_dir), i.status)
    };
    if status != Status::Offline {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor antes de desfazer uma ação.",
        ));
    }
    let token = req.token.trim();
    let backup_trash = trash_root(&base).join("backups").join(token);
    if backup_trash.is_dir() {
        let mut entries = tokio::fs::read_dir(&backup_trash).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir a lixeira: {e}"),
            )
        })?;
        let mut restored = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir a lixeira: {e}"),
            )
        })? {
            let name = entry.file_name();
            let destination = backups_dir(&base).join(&name);
            if destination.exists() {
                return Err(err(
                    StatusCode::CONFLICT,
                    "Já existe um arquivo com esse nome. Renomeie-o antes de desfazer.",
                ));
            }
            tokio::fs::rename(entry.path(), destination)
                .await
                .map_err(|e| {
                    err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Falha ao restaurar o backup: {e}"),
                    )
                })?;
            restored.push(name.to_string_lossy().into_owned());
        }
        let _ = tokio::fs::remove_dir_all(&backup_trash).await;
        app.0
            .lock()
            .await
            .push("Backup restaurado da lixeira.", "success");
        return Ok(Json(json!({"ok": true, "restored": restored})));
    }

    let content_trash = trash_root(&base).join("content").join(token);
    if !content_trash.is_dir() {
        return Err(err(
            StatusCode::NOT_FOUND,
            "Esta ação já foi desfeita ou expirou.",
        ));
    }
    let removed_path = content_trash.join("removed.json");
    let removed: Vec<InstalledContent> = tokio::fs::read(&removed_path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut kinds = tokio::fs::read_dir(&content_trash).await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao abrir a lixeira: {e}"),
        )
    })?;
    let mut restored = Vec::new();
    while let Some(kind_entry) = kinds.next_entry().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao abrir a lixeira: {e}"),
        )
    })? {
        if !kind_entry
            .file_type()
            .await
            .map(|t| t.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let kind = kind_entry.file_name().to_string_lossy().into_owned();
        let Some(destination_dir) = content_dir(&base, &kind) else {
            continue;
        };
        tokio::fs::create_dir_all(&destination_dir)
            .await
            .map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Falha ao restaurar conteúdo: {e}"),
                )
            })?;
        let mut files = tokio::fs::read_dir(kind_entry.path()).await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir a lixeira: {e}"),
            )
        })?;
        while let Some(file) = files.next_entry().await.map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao abrir a lixeira: {e}"),
            )
        })? {
            let name = file.file_name();
            let destination = destination_dir.join(&name);
            if destination.exists() {
                return Err(err(
                    StatusCode::CONFLICT,
                    format!(
                        "O arquivo {} já existe. Remova-o antes de desfazer.",
                        name.to_string_lossy()
                    ),
                ));
            }
            tokio::fs::rename(file.path(), destination)
                .await
                .map_err(|e| {
                    err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Falha ao restaurar conteúdo: {e}"),
                    )
                })?;
            restored.push(name.to_string_lossy().into_owned());
        }
    }
    let mut installed = read_installed_content(&base).await;
    for record in removed {
        if !installed
            .iter()
            .any(|item| item.kind == record.kind && item.filename == record.filename)
        {
            installed.push(record);
        }
    }
    write_installed_content(&base, &installed).await?;
    let _ = tokio::fs::remove_dir_all(&content_trash).await;
    app.0
        .lock()
        .await
        .push("Conteúdo restaurado da lixeira.", "success");
    Ok(Json(json!({"ok": true, "restored": restored})))
}

#[derive(Deserialize)]
struct BackupRestoreRequest {
    name: String,
}

async fn restore_backup(
    State(app): State<AppState>,
    Json(req): Json<BackupRestoreRequest>,
) -> Result<Json<Value>, ApiErr> {
    let (base, auto_backup, status, installing) = {
        let i = app.0.lock().await;
        (
            expand(&i.cfg.server_dir),
            i.cfg.auto_backup,
            i.status,
            i.installing,
        )
    };
    if status != Status::Offline || installing {
        return Err(err(
            StatusCode::CONFLICT,
            "Pare o servidor e aguarde instalações antes de restaurar um backup.",
        ));
    }
    let name = safe_filename(req.name.trim())
        .filter(|name| name.ends_with(".zip"))
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Backup inválido."))?;
    let path = backups_dir(&base).join(name);
    if !path.is_file() {
        return Err(err(StatusCode::NOT_FOUND, "Backup não encontrado."));
    }
    if auto_backup {
        let safety = format!(
            "antes-de-restaurar-{}.zip",
            Local::now().format("%Y%m%d-%H%M%S")
        );
        write_backup_file(&base, &backups_dir(&base).join(&safety)).await?;
        app.0
            .lock()
            .await
            .push(format!("Backup de segurança criado: {safety}"), "info");
    }
    extract_zip(&path, &base, vec![]).await?;
    app.0
        .lock()
        .await
        .push(format!("Backup restaurado: {name}"), "success");
    Ok(Json(json!({
        "ok": true,
        "message": "Backup restaurado. Inicie o servidor para conferir.",
        "items": backup_items(&base).await?,
    })))
}

#[derive(Deserialize)]
struct AutoBackupRequest {
    enabled: bool,
}

async fn set_auto_backup(
    State(app): State<AppState>,
    Json(req): Json<AutoBackupRequest>,
) -> ApiResult {
    let mut i = app.0.lock().await;
    i.cfg.auto_backup = req.enabled;
    i.persist().await.map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Falha ao salvar configuração: {e}"),
        )
    })?;
    i.push(
        if req.enabled {
            "Backups automáticos ativados antes de instalar conteúdo."
        } else {
            "Backups automáticos desativados."
        },
        "info",
    );
    Ok(Json(view(&mut i)))
}

#[derive(Deserialize)]
struct BackupNameQuery {
    name: String,
}

async fn download_backup(
    State(app): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<BackupNameQuery>,
) -> Result<Response, ApiErr> {
    let base = {
        let i = app.0.lock().await;
        expand(&i.cfg.server_dir)
    };
    let name = safe_filename(query.name.trim())
        .filter(|name| name.ends_with(".zip"))
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Backup inválido."))?;
    let path = backups_dir(&base).join(name);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("Backup não encontrado: {e}")))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(bytes))
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Falha ao preparar o backup: {e}"),
            )
        })
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
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if app.0.lock().await.status == Status::Offline {
            return;
        }
    }
    let sender = {
        let mut i = app.0.lock().await;
        if i.status == Status::Stopping {
            let sender = i.kill_tx.take();
            if sender.is_some() {
                i.push("Encerrando o servidor à força.", "warning");
            }
            sender
        } else {
            None
        }
    };
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if app.0.lock().await.status == Status::Offline {
            return;
        }
    }
}

async fn shutdown(State(app): State<AppState>) -> Json<Value> {
    let state = app.clone();
    tokio::spawn(async move {
        shutdown_server(&state).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        std::process::exit(0);
    });
    Json(json!({"ok": true}))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg_path = config_path();
    let mut cfg = load_config(&cfg_path).await;

    let loaded = load_e4all(&cfg.e4all.minecraft_dir);
    cfg.e4all = loaded;

    if detect_launch(&expand(&cfg.server_dir)).is_some() {
        cfg.loader = infer_loader(&expand(&cfg.server_dir));
    }

    let profiles = load_profiles(&cfg_path, &cfg).await;
    let java_major = detect_java_major(&cfg.java_path).await;
    let java_detected = java_major.is_some();
    let app_state = AppState(Arc::new(Mutex::new(Inner::new(
        cfg,
        profiles,
        cfg_path,
        java_detected,
        java_major,
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
        .route(
            "/api/server/properties",
            get(get_server_properties).post(save_server_properties),
        )
        .route("/api/server/icon", post(save_server_icon))
        .route("/api/server/open-folder", post(open_server_folder))
        .route("/api/server/next-port", post(next_port))
        .route("/api/java/ensure", post(java_ensure))
        .route("/api/network/diagnose", get(network_diagnose))
        .route("/api/minecraft/versions", get(minecraft_versions))
        .route("/api/server/create", post(create_server))
        .route("/api/content", get(list_content))
        .route("/api/content/delete", post(delete_content))
        .route("/api/content/search", post(search_content))
        .route("/api/content/versions", get(get_content_versions))
        .route("/api/content/install", post(install_content))
        .route("/api/curseforge/status", get(curseforge_status))
        .route("/api/curseforge/config", post(save_curseforge_config))
        .route("/api/modpacks/search", post(search_modpacks))
        .route("/api/modpacks/versions", get(modpack_versions))
        .route("/api/modpacks/install", post(install_modpack))
        .route("/api/backups", get(list_backups))
        .route("/api/backups/create", post(create_backup))
        .route("/api/backups/delete", post(delete_backup))
        .route("/api/backups/download", get(download_backup))
        .route("/api/backups/restore", post(restore_backup))
        .route("/api/backups/auto", post(set_auto_backup))
        .route("/api/trash/restore", post(restore_trash))
        .route("/api/whitelist", get(get_whitelist))
        .route("/api/whitelist/add", post(add_whitelist))
        .route("/api/whitelist/remove", post(remove_whitelist))
        .route("/api/whitelist/toggle", post(toggle_whitelist))
        .route("/api/update/check", get(check_update))
        .route("/api/update/install", post(install_update))
        .route("/api/update/rollback", post(rollback_update))
        .route("/api/shutdown", post(shutdown))
        .route("/api/config", post(save_config))
        .route("/api/e4all/config", post(save_e4all))
        .route("/api/e4all/status", post(e4all_status))
        .route("/api/e4all/load", post(e4all_load))
        .route("/api/e4all/link", post(e4all_link))
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

#[cfg(test)]
mod tests {
    use super::{
        extract_e4all_link, java_major_for_version, offline_uuid, parse_java_major, qr_svg,
        update_checksum_from_sums, version_parts,
    };

    #[test]
    fn extracts_assigned_e4all_domain() {
        assert_eq!(
            extract_e4all_link("[Render thread/INFO] [e4all]: Domain assigned: abcde.e4mc.link"),
            Some("abcde.e4mc.link".into())
        );
    }

    #[test]
    fn extracts_reassigned_e4all_domain() {
        assert_eq!(
            extract_e4all_link("Domain reassigned: new.e4mc.link (was: old.e4mc.link)"),
            Some("new.e4mc.link".into())
        );
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert_eq!(extract_e4all_link("Domain assigned: exemplo.com"), None);
    }

    #[test]
    fn qr_svg_contains_payload_modules() {
        let svg = qr_svg("https://abc.e4mc.link").expect("qr");
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("shape-rendering=\"crispEdges\""));
        assert!(svg.contains("<path d=\"M"));
    }

    #[test]
    fn offline_uuid_is_v3_and_deterministic() {
        let uuid = offline_uuid("Notch");
        assert_eq!(uuid, offline_uuid("Notch"));
        assert_eq!(&uuid[14..15], "3");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(uuid.len(), 36);
    }

    #[test]
    fn picks_java_for_minecraft_versions() {
        assert_eq!(java_major_for_version("26.1"), 25);
        assert_eq!(java_major_for_version("26.2"), 25);
        assert_eq!(java_major_for_version("26.3"), 25);
        assert_eq!(java_major_for_version("1.21.11"), 21);
        assert_eq!(java_major_for_version("1.20.5"), 21);
        assert_eq!(java_major_for_version("1.20.4"), 17);
        assert_eq!(java_major_for_version("1.16.5"), 8);
        assert_eq!(java_major_for_version("desconhecida"), 21);
    }

    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_java_major("openjdk version \"1.8.0_402\""), Some(8));
        assert_eq!(parse_java_major("openjdk version \"17.0.10\""), Some(17));
        assert_eq!(parse_java_major("openjdk version \"21.0.4\""), Some(21));
        assert_eq!(parse_java_major("sem número"), None);
    }

    #[test]
    fn compares_update_versions() {
        assert!(version_parts("0.10.0") > version_parts("0.9.9"));
        assert!(version_parts("v1.0.0") > version_parts("0.99.99"));
        assert_eq!(version_parts("0.8.3"), (0, 8, 3));
    }

    #[test]
    fn picks_checksum_from_sums_file() {
        let sums = "aaa111  ShadowPainel-0.9.0.AppImage\nbbb222  outro.zip\n";
        assert_eq!(
            update_checksum_from_sums(sums, "ShadowPainel-0.9.0.AppImage"),
            Some("aaa111".into())
        );
        assert_eq!(update_checksum_from_sums(sums, "sumiu"), None);
    }
}
