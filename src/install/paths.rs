use anyhow::{Context, Result};
use std::path::PathBuf;

pub struct ZedPaths {
    pub config_dir: PathBuf,
    pub tasks: PathBuf,
    pub keymap: PathBuf,
}

impl ZedPaths {
    pub fn detect() -> Result<Self> {
        let home = dirs::home_dir().context("no home directory")?;
        let config_dir = home.join(".config").join("zed");
        Ok(Self {
            tasks: config_dir.join("tasks.json"),
            keymap: config_dir.join("keymap.json"),
            config_dir,
        })
    }
}
