use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{WebContext, WebViewBuilder};

#[cfg(target_os = "linux")]
use tao::platform::unix::WindowExtUnix;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;

fn candidates(exe_dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|name| exe_dir.join(name)).collect()
}

fn find_file(paths: Vec<PathBuf>) -> io::Result<PathBuf> {
    paths
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "núcleo do ShadowPainel não encontrado",
            )
        })
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn webview_data_dir() -> io::Result<PathBuf> {
    let base = if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("APPDATA não está disponível"))?
    } else if cfg!(target_os = "macos") {
        home_dir()
            .ok_or_else(|| io::Error::other("HOME não está disponível"))?
            .join("Library")
            .join("Application Support")
    } else if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
        PathBuf::from(xdg)
    } else {
        home_dir()
            .ok_or_else(|| io::Error::other("HOME não está disponível"))?
            .join(".local")
            .join("share")
    };
    let path = base.join("ShadowPainel").join("WebView2");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn start_core(exe_dir: &Path) -> Result<(Child, String), Box<dyn Error>> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let core = find_file(candidates(
        exe_dir,
        &[
            &format!("backend/shadowpainel-core{suffix}"),
            &format!("../lib/shadowpainel/shadowpainel-core{suffix}"),
            &format!("../../backend/target/release/shadowpainel{suffix}"),
            &format!("../../../backend/target/release/shadowpainel{suffix}"),
        ],
    ))?;

    let frontend = find_file(candidates(
        exe_dir,
        &[
            "frontend/dist/index.html",
            "../share/shadowpainel/frontend/index.html",
            "../../frontend/dist/index.html",
            "../../../frontend/dist/index.html",
        ],
    ))?;
    let frontend_dir = frontend
        .parent()
        .ok_or_else(|| io::Error::other("pasta do frontend inválida"))?;

    let mut child = Command::new(&core)
        .env("SHADOWPAINEL_FRONTEND_DIR", frontend_dir)
        .env(
            "SHADOWPAINEL_ADDR",
            env::var("SHADOWPAINEL_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into()),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("não foi possível ler o núcleo"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    for _ in 0..100 {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("ShadowPainel disponível em ") {
            let address = value.trim();
            let url = if address.starts_with("http://") || address.starts_with("https://") {
                address.to_string()
            } else {
                format!("http://{address}")
            };
            if url.contains("://") && url.contains(':') && !url.ends_with(':') {
                return Ok((child, url));
            }
        }
    }

    let _ = child.kill();
    Err(io::Error::other("o núcleo não informou uma porta disponível").into())
}

fn request_shutdown(url: &str) {
    let Some(address) = url.strip_prefix("http://") else {
        return;
    };
    let address = address.split('/').next().unwrap_or(address);
    let Ok(mut stream) = TcpStream::connect(address) else {
        return;
    };
    let request = format!(
        "POST /api/shutdown HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.write_all(request.as_bytes());
    let _ = stream.flush();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut response = [0u8; 256];
    let _ = stream.read(&mut response);
}

fn main() -> Result<(), Box<dyn Error>> {
    let exe_dir = env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("diretório do aplicativo inválido"))?
        .to_path_buf();

    let (mut core, url) = start_core(&exe_dir)?;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("ShadowPainel — Minecraft Server Control")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 900.0))
        .build(&event_loop)?;

    // Abre em tela cheia
    window.set_maximized(true);

    let webview_data = webview_data_dir()?;
    let mut web_context = WebContext::new(Some(webview_data));
    let builder = WebViewBuilder::with_web_context(&mut web_context).with_url(&url);

    #[cfg(not(target_os = "linux"))]
    let _webview = builder.build(&window)?;

    #[cfg(target_os = "linux")]
    let _webview = {
        let vbox = window
            .default_vbox()
            .ok_or_else(|| io::Error::other("GTK vbox não disponível"))?;
        builder.build_gtk(vbox)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                request_shutdown(&url);
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    match core.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(100));
                        }
                        _ => {
                            let _ = core.kill();
                            let _ = core.wait();
                            break;
                        }
                    }
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
