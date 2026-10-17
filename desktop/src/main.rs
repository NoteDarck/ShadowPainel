use std::{
    env,
    error::Error,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

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
    eprintln!("[desktop] backend encontrado em: {}", core.display());

    let frontend = find_file(candidates(
        exe_dir,
        &[
            "frontend/dist/index.html",
            "../share/shadowpainel/frontend/index.html",
            "../../frontend/dist/index.html",
            "../../../frontend/dist/index.html",
        ],
    ))?;
    eprintln!("[desktop] frontend encontrado em: {}", frontend.display());

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

    // Tenta por até 10 segundos (100 x 100ms)
    for attempt in 0..100 {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let trimmed = line.trim();
        eprintln!("[desktop] backend disse: {trimmed}");

        if let Some(value) = trimmed.strip_prefix("ShadowPainel disponível em ") {
            // O backend imprime "http://127.0.0.1:PORTA" (com http:// já incluso)
            // ou "127.0.0.1:PORTA". Normaliza os dois casos.
            let address = value.trim();
            let url = if address.starts_with("http://") || address.starts_with("https://") {
                address.to_string()
            } else {
                format!("http://{address}")
            };

            // Validação rígida: precisa ter host e porta
            if url.contains("://") && url.contains(':') && !url.ends_with(':') {
                eprintln!("[desktop] URL final: {url}");
                return Ok((child, url));
            } else {
                eprintln!("[desktop] AVISO: endereço inválido recebido: {address:?}");
            }
        }

        if attempt == 99 {
            break;
        }
    }

    let _ = child.kill();
    Err(io::Error::other(
        "o núcleo não informou uma porta disponível (veja as linhas [desktop] acima)",
    )
    .into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let exe_dir = env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("diretório do aplicativo inválido"))?
        .to_path_buf();
    eprintln!("[desktop] diretório do executável: {}", exe_dir.display());

    let (mut core, url) = start_core(&exe_dir)?;
    eprintln!("[desktop] carregando URL na janela: {url}");

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("ShadowPainel — Minecraft Server Control")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 900.0))
        .build(&event_loop)?;

    let builder = WebViewBuilder::new().with_url(&url);

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
                let _ = core.kill();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
