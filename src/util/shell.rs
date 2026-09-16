use anyhow::{Context, Result};
use std::process::Command;

pub fn exec_shell(command: &str, args: &[String]) -> Result<()> {
    Command::new(command)
        .args(args)
        .spawn()
        .context("Command Failed!")?;
    Ok(())
}
