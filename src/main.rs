use ai_quota_zectrix_dashboard::{model::Dashboard, render};
use clap::{Parser, Subcommand};
use std::{error::Error, fs, path::PathBuf};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Select a NOTE4 page and confirm its first upload.
    Setup,
    /// Render a normalized dashboard fixture to a 1-bit NOTE4 PNG.
    Preview {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Setup => {
            use ai_quota_zectrix_dashboard::setup::{data_dir, run_setup};
            let dashboard = Dashboard {
                now: chrono::Local::now().fixed_offset(),
                screen_updated_at: None,
                codex: Default::default(),
                antigravity: Default::default(),
                codex_radar: Default::default(),
                antigravity_radar: Default::default(),
            };
            run_setup(&data_dir()?, &dashboard)?;
        }
        Command::Preview { input, output } => {
            let dashboard: Dashboard = serde_json::from_slice(&fs::read(input)?)?;
            let frame = render(&dashboard)?;
            fs::write(output, frame.png_bytes()?)?;
            println!("{}", frame.sha256());
        }
    }
    Ok(())
}
