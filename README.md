# ShadowPainel

MVP de um painel visual para gerenciar servidores Minecraft sem exigir terminal, Java ou arquivos `.jar` do usuário. O projeto foi adicionado em uma pasta própria para não alterar o jogo Godot existente em `Newfoundland`.

## Stack

- **Rust + Axum + Tokio**: núcleo HTTP local e ciclo de vida do servidor.
- **Svelte + TypeScript + Vite**: dashboard responsivo.
- **sysinfo**: métricas de CPU e memória.
- Futuro: processo Java real, SQLite, Tauri e empacotamento Windows/Linux.

## Rodar em desenvolvimento

```bash
cd frontend
npm install
npm run build
cd ../backend
cargo run
```

Abra `http://localhost:4173`.

O backend atual oferece uma simulação segura do ciclo de vida: iniciar/parar, logs, comandos, endereço e métricas. O próximo passo de produção é conectar `tokio::process::Command` ao Java detectado e persistir servidores em SQLite.

## Distribuição

O backend é Rust multiplataforma. Para Linux, gerar um binário e empacotar como AppImage; para Windows, compilar com `x86_64-pc-windows-msvc` e distribuir como `.exe`. A camada de frontend permanece a mesma nos dois sistemas.
