use std::{
    env,
    error::Error,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

fn candidates(exe_dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|name| exe_dir.join(name)).collect()
}

fn find_file(paths: Vec<PathBuf>) -> io::Result<PathBuf> {
    paths
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "núcleo do ShadowPainel não encontrado"))
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

    let mut child = Command::new(core)
        .env("SHADOWPAINEL_FRONTEND_DIR", frontend_dir)
        .env("SHADOWPAINEL_ADDR", env::var("SHADOWPAINEL_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into()))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("não foi possível ler o núcleo"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut address = None;
    for _ in 0..20 {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if let Some(value) = line.trim().strip_prefix("ShadowPainel disponível em ") {
            address = Some(value.to_string());
            break;
        }
    }
    if let Some(address) = address {
        Ok((child, format!("http://{address}")))
    } else {
        let _ = child.kill();
        Err(io::Error::other("o núcleo não informou uma porta disponível").into())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // O WebKitGTK usado pelo wry precisa de um handle X11 no Linux.
    // Em sessões Wayland, o XWayland fornece essa compatibilidade.
    #[cfg(target_os = "linux")]
    {
        env::set_var("GDK_BACKEND", "x11");
        env::set_var("WINIT_UNIX_BACKEND", "x11");
    }

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
    let _webview = WebViewBuilder::new().with_url(&url).build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let _ = core.kill();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
