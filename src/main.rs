use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod daemon;
mod doctor;
mod fork;
mod install;
mod logging;
mod selection;
mod send;
mod util;

#[derive(Parser)]
#[command(name = "zcc", version, about = "Zed <-> Claude Code integration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Send the current editor selection to Claude Code. Invoked by the Zed task.
    Send {
        #[arg(long)]
        worktree: PathBuf,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        row: u32,
        /// Selection text. If --selection-stdin is set, this is ignored and stdin is read instead.
        #[arg(long, default_value = "")]
        selection: String,
        /// Read the selection from stdin instead of --selection. Avoids shell-quoting issues.
        #[arg(long)]
        selection_stdin: bool,
    },
    /// Run the per-worktree WebSocket daemon. Not invoked by users directly.
    Daemon {
        #[arg(long)]
        worktree: PathBuf,
    },
    /// Merge zcc's entries into the global Zed tasks.json and keymap.json.
    Install,
    /// Remove zcc's entries from the global Zed config files.
    Uninstall,
    /// Check the installation and print diagnostics.
    Doctor,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sub_name = match &cli.command {
        Command::Send { .. } => "send",
        Command::Daemon { .. } => "daemon",
        Command::Install => "install",
        Command::Uninstall => "uninstall",
        Command::Doctor => "doctor",
    };
    let _ = logging::init(sub_name);

    match cli.command {
        Command::Send { worktree, file, row, selection, selection_stdin } => {
            let sel = if selection_stdin {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                selection
            };
            send::run(worktree, file, row, sel)
        }
        Command::Daemon { worktree } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(daemon::run(worktree))
        }
        Command::Install => install::run(),
        Command::Uninstall => install::uninstall(),
        Command::Doctor => doctor::run(),
    }
}
