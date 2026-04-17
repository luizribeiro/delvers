mod client;
mod dungeon;
mod entity;
mod game;
mod protocol;
mod server;
mod world;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

#[derive(Parser, Debug)]
#[command(name = "godwars.ai", about = "A cooperative multi-player roguelike")]
struct Cli {
    /// Player name (for client mode)
    #[arg(short = 'n', long)]
    name: Option<String>,

    /// Path to the UNIX socket
    #[arg(short = 's', long)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run as server only (listen on socket)
    Server,
    /// Run as client only (don't try to auto-spawn server)
    Client,
}

fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("GODWARS_SOCKET") {
        return PathBuf::from(p);
    }
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    dir.join("godwars.sock")
}

fn pick_name() -> String {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    format!("wanderer{}", std::process::id() % 1000)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli
        .socket
        .unwrap_or_else(default_socket_path)
        .to_string_lossy()
        .to_string();

    match cli.command {
        Some(Cmd::Server) => run_async(async move { server::run(&socket).await }),
        Some(Cmd::Client) => {
            let name = cli.name.unwrap_or_else(pick_name);
            run_async(async move { client::run(&socket, &name).await })
        }
        None => {
            // Auto mode: connect, else spawn server + connect.
            let name = cli.name.unwrap_or_else(pick_name);
            if !can_connect(&socket) {
                spawn_server(&socket)?;
                wait_for_socket(&socket, Duration::from_secs(3))?;
            }
            run_async(async move { client::run(&socket, &name).await })
        }
    }
}

fn run_async<F: std::future::Future<Output = Result<()>>>(fut: F) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(fut)
}

fn can_connect(socket: &str) -> bool {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(socket))
            .await
            .ok()
            .and_then(|r| r.ok())
            .is_some()
    })
}

fn spawn_server(socket: &str) -> Result<()> {
    let exe = std::env::current_exe()?;
    let log_path = std::env::var("GODWARS_LOG").unwrap_or_else(|_| "/tmp/godwars.log".into());
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log_file.try_clone()?;
    Command::new(exe)
        .arg("--socket")
        .arg(socket)
        .arg("server")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .spawn()?;
    Ok(())
}

fn wait_for_socket(socket: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if can_connect(socket) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    anyhow::bail!("timed out waiting for server at {}", socket);
}
