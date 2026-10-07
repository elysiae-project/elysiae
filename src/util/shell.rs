use anyhow::{Context, Result};
use std::process::Command;

pub fn exec_shell(command: &str, args: &[String]) -> Result<std::process::Child> {
    Command::new(command)
        .args(args)
        .spawn()
        .context("Command Failed!")
}
