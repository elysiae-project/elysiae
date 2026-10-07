use anyhow::{Context, Result};
use std::process::Command;

pub fn exec_shell(command: &str, args: &[String]) -> Result<std::process::Child> {
    Command::new(command)
        .args(args)
        .spawn()
        .context("Command Failed!")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_existing_command() {
        let mut child = exec_shell("true", &[]).unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn rejects_missing_command() {
        assert!(exec_shell("elysiae-command-that-does-not-exist", &[]).is_err());
    }
}
