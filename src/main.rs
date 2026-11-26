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
        Command::Preview { input, output } => {
            let dashboard: Dashboard = serde_json::from_slice(&fs::read(input)?)?;
            let frame = render(&dashboard)?;
            fs::write(output, frame.png_bytes()?)?;
            println!("{}", frame.sha256());
        }
    }
    Ok(())
}
