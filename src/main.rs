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
#[command(name = "zcc", about = "Zed <-> Claude Code integration")]
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
        /// Selection text. Used only if neither --read-clipboard nor --selection-stdin is set.
        #[arg(long, default_value = "")]
        selection: String,
        /// Read the selection from stdin. Lower priority than --read-clipboard.
        #[arg(long)]
        selection_stdin: bool,
        /// Read the selection from the system clipboard via NSPasteboard. Highest
        /// priority — preferred over --selection-stdin and --selection.
        #[arg(long)]
        read_clipboard: bool,
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
    /// Print the zcc version.
    Version,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sub_name = match &cli.command {
        Command::Send { .. } => "send",
        Command::Daemon { .. } => "daemon",
        Command::Install => "install",
        Command::Uninstall => "uninstall",
        Command::Doctor => "doctor",
        Command::Version => "version",
    };
    let _ = logging::init(sub_name);

    match cli.command {
        Command::Send {
            worktree,
            file,
            row,
            selection,
            selection_stdin,
            read_clipboard,
        } => {
            let sel = if read_clipboard {
                let mut cb = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("open clipboard: {e}"))?;
                cb.get_text()
                    .map_err(|e| anyhow::anyhow!("read clipboard text: {e}"))?
            } else if selection_stdin {
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
        Command::Version => {
            println!("zcc {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
