use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod proxy;
mod common;
mod tsukuyomi;
mod validate_replay;

#[derive(Parser)]
#[command(name = "ygopro-toolkits")]
#[command(about = "CLI tools for ygopro")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate a replay file by replaying it through the engine.
    ValidateReplay {
        /// Replay (.yrp) files to validate
        #[arg(required = true)]
        path: Vec<PathBuf>,
        /// Wait for a viewer to connect on this port before replaying responses
        #[arg(long)]
        wait: Option<u16>,
        /// Validation timeout in seconds
        #[arg(long, default_value_t = 5)]
        timeout: u64,
        /// ygopro-server binary to validate against instead of the in-process engine
        #[arg(long, requires = "server_cwd")]
        server_bin: Option<PathBuf>,
        /// Working directory the ygopro-server is launched in (defaults to the directory containing the server binary)
        #[arg(long, requires = "server_bin")]
        server_cwd: Option<PathBuf>,
    },
    /// Replay takeover arena. Serve a replay's terminal scene to two takeover players, forever.
    Tsukuyomi {
        /// Replay (.yrp) file to build the scene from
        path: PathBuf,
        /// Listen port for the takeover players
        #[arg(short, long, default_value_t = 7911)]
        port: u16,
        /// ygopro-server binary to validate against instead of the in-process engine
        #[arg(long, requires = "server_cwd")]
        server_bin: Option<PathBuf>,
        /// Working directory the ygopro-server is launched in
        #[arg(long, requires = "server_bin")]
        server_cwd: Option<PathBuf>,
    },
    /// Logging proxy middleware.
    Proxy {
        /// Proxy target
        #[arg(short, long)]
        target: SocketAddr,
        /// Proxy listening on port
        #[arg(short, long, default_value_t = 8911)]
        port: u32,
    },
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::ValidateReplay { path, wait, timeout, server_bin, server_cwd } => {
            validate_replay::run(&path, wait, timeout, server_bin, server_cwd).await;
        }
        Commands::Tsukuyomi { path, port, server_bin, server_cwd } => {
            tsukuyomi::run(&path, port, server_bin, server_cwd).await;
        }
        Commands::Proxy { target, port } => {
            proxy::run(target, port).await;
        }
    }
}
