use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use jit_dispatch::{Config, Orchestrator};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "jit-dispatch")]
#[command(about = "Orchestrator for jit issue tracker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the orchestrator daemon
    Start {
        /// Path to configuration file
        #[arg(short, long, default_value = "dispatch.toml")]
        config: PathBuf,

        /// Path to jit repository
        #[arg(short, long, default_value = ".")]
        repo: PathBuf,

        /// Run once and exit (don't loop)
        #[arg(long)]
        once: bool,
    },

    /// Run one dispatch cycle
    Once {
        /// Path to configuration file
        #[arg(short, long, default_value = "dispatch.toml")]
        config: PathBuf,

        /// Path to jit repository
        #[arg(short, long, default_value = ".")]
        repo: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { config, repo, once } => {
            let config = Config::from_file(&config)
                .context(format!("Failed to load config from {:?}", config))?;

            println!("Starting jit-dispatch orchestrator...");
            println!("  Config: {:?}", config);
            println!("  Repository: {:?}", repo);
            println!("  Poll interval: {}s", config.poll_interval_secs);
            println!("  Agents: {}", config.agents.len());

            let mut orchestrator = Orchestrator::with_config(&repo, config.clone());

            loop {
                match orchestrator.run_dispatch_cycle() {
                    Ok(assigned) => {
                        if assigned > 0 {
                            println!("Assigned {} issue(s)", assigned);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error during dispatch cycle: {}", e);
                    }
                }

                if once {
                    break;
                }

                thread::sleep(Duration::from_secs(config.poll_interval_secs));
            }

            Ok(())
        }

        Commands::Once { config, repo } => {
            let config = Config::from_file(&config)
                .context(format!("Failed to load config from {:?}", config))?;

            let mut orchestrator = Orchestrator::with_config(&repo, config);

            let assigned = orchestrator
                .run_dispatch_cycle()
                .context("Failed to run dispatch cycle")?;

            println!("Assigned {} issue(s)", assigned);

            Ok(())
        }
    }
}
