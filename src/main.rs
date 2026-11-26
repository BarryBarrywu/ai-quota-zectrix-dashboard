use ai_quota_zectrix_dashboard::{
    daemon::{Observers, diagnostics, run_local},
    render,
    setup::{data_dir, run_setup},
};
use clap::{Parser, Subcommand};
use std::{error::Error, fs, path::PathBuf};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Observe both providers and publish changed frames to the configured NOTE4 page.
    Run {
        #[arg(long, default_value = "codex")]
        codex_program: PathBuf,
    },
    /// Render both live quotas and public reset signals to a local PNG.
    LivePreview {
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "codex")]
        codex_program: PathBuf,
    },
    /// Select a NOTE4 page and confirm its first upload.
    Setup {
        #[arg(long, default_value = "codex")]
        codex_program: PathBuf,
    },
    /// Report the health of each observation source and the publisher.
    Diagnostics,
    /// Manage the current user’s macOS LaunchAgent.
    LaunchAgent {
        #[command(subcommand)]
        action: ai_quota_zectrix_dashboard::launch_agent::AgentAction,
    },
    /// Render a normalized dashboard fixture to a 1-bit NOTE4 PNG.
    Preview {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
fn main() -> Result<(), Box<dyn Error>> {
    let (dashboard, output) = match Cli::parse().command {
        Command::LaunchAgent { action } => {
            return ai_quota_zectrix_dashboard::launch_agent::manage(action);
        }
        Command::Diagnostics => {
            println!("{}", diagnostics(&data_dir()?)?);
            return Ok(());
        }
        Command::Run { codex_program } => {
            return run_local(Observers::local(&data_dir()?, &codex_program));
        }
        Command::Setup { codex_program } => {
            let dir = data_dir()?;
            return run_setup(&dir, &Observers::local(&dir, &codex_program).live()?);
        }
        Command::Preview { input, output } => (serde_json::from_slice(&fs::read(input)?)?, output),
        Command::LivePreview {
            output,
            codex_program,
        } => (
            Observers::local(&data_dir()?, &codex_program).live()?,
            output,
        ),
    };
    let frame = render(&dashboard)?;
    fs::write(output, frame.png_bytes()?)?;
    println!("{}", frame.sha256());
    Ok(())
}
