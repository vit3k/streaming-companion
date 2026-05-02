use tokio::process::Command;

pub async fn suspend_system() -> Result<(), String> {
    let systemctl = run_suspend_command("systemctl").await;
    if systemctl.is_ok() {
        return Ok(());
    }

    let loginctl = run_suspend_command("loginctl").await;
    if loginctl.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "both suspend commands failed (systemctl: {}, loginctl: {})",
            systemctl.err().unwrap_or_else(|| "unknown error".to_string()),
            loginctl.err().unwrap_or_else(|| "unknown error".to_string())
        ))
    }
}

async fn run_suspend_command(command: &str) -> Result<(), String> {
    let status = Command::new(command)
        .arg("suspend")
        .status()
        .await
        .map_err(|error| format!("{command} execution failed: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{command} exited with status {status}"))
    }
}
