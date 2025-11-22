use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;
mod completion;
mod config;
mod db;
mod executor;
mod export;
mod repl;

use common::{DB_FILENAME, DEFAULT_MAX_OUTPUT_SIZE, CAHIER_DIR};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start cahier REPL (default)
    Start {
        /// Maximum output size in bytes before redirecting to file (default: 16384)
        #[arg(long, default_value_t = DEFAULT_MAX_OUTPUT_SIZE)]
        max_output_size: usize,
    },
    /// Export history to markdown
    Export {
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,

        /// Export only the commands (plain text)
        #[arg(long)]
        only_commands: bool,
    },
}

fn main() -> Result<()> {
    // Ensure cahier directory exists
    std::fs::create_dir_all(CAHIER_DIR)?;

    let args = Args::parse();
    let db = db::Database::init(DB_FILENAME)?;
    let config = config::Config::load()?;

    match args.command {
        Some(Commands::Export {
            output,
            only_commands,
        }) => {
            let content = if only_commands {
                export::generate_commands_text(&db)?
            } else {
                export::generate_markdown(&db)?
            };

            if let Some(path) = output {
                std::fs::write(path, content)?;
            } else {
                println!("{}", content);
            }
            Ok(())
        }
        Some(Commands::Start { max_output_size }) => {
            let pty_writer = setup_signal_handler()?;
            repl::run_repl(db, max_output_size, pty_writer, config)
        }
        None => {
            // Default behavior: start REPL with default max_output_size
            let pty_writer = setup_signal_handler()?;
            repl::run_repl(db, DEFAULT_MAX_OUTPUT_SIZE, pty_writer, config)
        }
    }
}

/// Sets up the Ctrl+C signal handler and returns the shared PTY writer
fn setup_signal_handler() -> Result<Arc<Mutex<Option<Box<dyn Write + Send>>>>> {
    let pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));

    let writer_clone = Arc::clone(&pty_writer);
    ctrlc::set_handler(move || {
        if let Ok(mut writer_opt) = writer_clone.lock() {
            if let Some(writer) = writer_opt.as_mut() {
                // Send Ctrl+C (ETX) to the running command
                let _ = writer.write_all(&[3]);
                let _ = writer.flush();
            }
            // If no writer, do nothing (at prompt)
        }
    })?;

    Ok(pty_writer)
}
