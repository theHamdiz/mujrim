use std::process::Command;

pub fn run(cmd: &str, args: &[&str], env: &[(&str, &str)]) -> Result<(), String> {
    let mut command = Command::new(cmd);
    command.args(args);
    for (k, v) in env {
        command.env(k, v);
    }
    let status = command
        .status()
        .map_err(|e| format!("failed to execute {cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} exited with status {status}"))
    }
}

pub fn output(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd} exited with status {}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("invalid utf-8 output from {cmd}: {e}"))
}
