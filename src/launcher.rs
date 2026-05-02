use std::env;
use std::path::PathBuf;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::process::{launch_id_candidates, wait_for_appid_process};

pub async fn launch_game_with_steam(
    id: &str,
    wait_for_start: bool,
    wait_timeout_ms: u64,
) -> Result<Value, String> {
    let candidate_ids = launch_id_candidates(id);
    let mut attempts: Vec<Value> = Vec::new();

    for script in launcher_script_candidates() {
        if !script.is_file() {
            continue;
        }

        let script_string = script.to_string_lossy().to_string();
        if run_attempt(
            "bash",
            vec![script_string.clone(), id.to_string()],
            format!("wrapper:{}", script_string),
            &mut attempts,
        )
        .await
        {
            if wait_for_start {
                let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                attempts.push(json!({
                    "label": "verify:wrapper",
                    "success": verified,
                    "timeout_ms": wait_timeout_ms,
                }));
                if verified {
                    return Ok(json!({
                        "method": "wrapper",
                        "wait_requested": wait_for_start,
                        "verified": true,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            } else {
                return Ok(json!({
                    "method": "wrapper",
                    "wait_requested": wait_for_start,
                    "verified": Value::Null,
                    "candidate_ids": candidate_ids,
                    "attempts": attempts,
                }));
            }
        }
    }

    for candidate_id in &candidate_ids {
        let uris = [
            format!("steam://rungameid/{candidate_id}"),
            format!("steam://launch/{candidate_id}"),
        ];

        for uri in &uris {
            if run_attempt(
                "xdg-open",
                vec![uri.clone()],
                format!("xdg-open:{uri}"),
                &mut attempts,
            )
            .await
            {
                if wait_for_start {
                    let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                    attempts.push(json!({
                        "label": "verify:xdg-open",
                        "success": verified,
                        "timeout_ms": wait_timeout_ms,
                        "uri": uri,
                    }));
                    if verified {
                        return Ok(json!({
                            "method": "xdg-open",
                            "wait_requested": wait_for_start,
                            "verified": true,
                            "candidate_ids": candidate_ids,
                            "attempts": attempts,
                        }));
                    }
                } else {
                    return Ok(json!({
                        "method": "xdg-open",
                        "wait_requested": wait_for_start,
                        "verified": Value::Null,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            }

            if run_attempt(
                "gio",
                vec!["open".to_string(), uri.clone()],
                format!("gio-open:{uri}"),
                &mut attempts,
            )
            .await
            {
                if wait_for_start {
                    let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                    attempts.push(json!({
                        "label": "verify:gio-open",
                        "success": verified,
                        "timeout_ms": wait_timeout_ms,
                        "uri": uri,
                    }));
                    if verified {
                        return Ok(json!({
                            "method": "gio-open",
                            "wait_requested": wait_for_start,
                            "verified": true,
                            "candidate_ids": candidate_ids,
                            "attempts": attempts,
                        }));
                    }
                } else {
                    return Ok(json!({
                        "method": "gio-open",
                        "wait_requested": wait_for_start,
                        "verified": Value::Null,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            }

            if run_attempt(
                "steam",
                vec![uri.clone()],
                format!("steam-uri:{uri}"),
                &mut attempts,
            )
            .await
            {
                if wait_for_start {
                    let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                    attempts.push(json!({
                        "label": "verify:steam-uri",
                        "success": verified,
                        "timeout_ms": wait_timeout_ms,
                        "uri": uri,
                    }));
                    if verified {
                        return Ok(json!({
                            "method": "steam-uri",
                            "wait_requested": wait_for_start,
                            "verified": true,
                            "candidate_ids": candidate_ids,
                            "attempts": attempts,
                        }));
                    }
                } else {
                    return Ok(json!({
                        "method": "steam-uri",
                        "wait_requested": wait_for_start,
                        "verified": Value::Null,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            }

            if run_attempt(
                "flatpak",
                vec![
                    "run".to_string(),
                    "com.valvesoftware.Steam".to_string(),
                    uri.clone(),
                ],
                format!("flatpak-steam-uri:{uri}"),
                &mut attempts,
            )
            .await
            {
                if wait_for_start {
                    let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                    attempts.push(json!({
                        "label": "verify:flatpak-steam-uri",
                        "success": verified,
                        "timeout_ms": wait_timeout_ms,
                        "uri": uri,
                    }));
                    if verified {
                        return Ok(json!({
                            "method": "flatpak-steam-uri",
                            "wait_requested": wait_for_start,
                            "verified": true,
                            "candidate_ids": candidate_ids,
                            "attempts": attempts,
                        }));
                    }
                } else {
                    return Ok(json!({
                        "method": "flatpak-steam-uri",
                        "wait_requested": wait_for_start,
                        "verified": Value::Null,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            }
        }

        if run_attempt(
            "steam",
            vec![
                "-applaunch".to_string(),
                "--".to_string(),
                candidate_id.clone(),
            ],
            format!("steam-applaunch:{candidate_id}"),
            &mut attempts,
        )
        .await
        {
            if wait_for_start {
                let verified = wait_for_appid_process(&candidate_ids, wait_timeout_ms).await;
                attempts.push(json!({
                    "label": "verify:steam-applaunch",
                    "success": verified,
                    "timeout_ms": wait_timeout_ms,
                    "id": candidate_id,
                }));
                if verified {
                    return Ok(json!({
                        "method": "steam-applaunch",
                        "wait_requested": wait_for_start,
                        "verified": true,
                        "candidate_ids": candidate_ids,
                        "attempts": attempts,
                    }));
                }
            } else {
                return Ok(json!({
                    "method": "steam-applaunch",
                    "wait_requested": wait_for_start,
                    "verified": Value::Null,
                    "candidate_ids": candidate_ids,
                    "attempts": attempts,
                }));
            }
        }
    }

    if wait_for_start {
        Err(format!(
            "launch command(s) were issued but no app start was verified within {} ms",
            wait_timeout_ms
        ))
    } else {
        Err(format!(
            "all launch methods failed: {}",
            attempts
                .iter()
                .map(|attempt| {
                    let label = attempt.get("label").and_then(Value::as_str).unwrap_or("unknown");
                    let success = attempt.get("success").and_then(Value::as_bool).unwrap_or(false);
                    format!("{label}={success}")
                })
                .collect::<Vec<String>>()
                .join(", ")
        ))
    }
}

pub fn launcher_script_candidates() -> Vec<PathBuf> {
    let mut scripts = Vec::new();

    if let Ok(path) = env::var("SUSPEND_WEB_LAUNCHER") {
        if !path.trim().is_empty() {
            scripts.push(PathBuf::from(path.trim()));
        }
    }

    if let Ok(cwd) = env::current_dir() {
        scripts.push(cwd.join("scripts/steam-launch.sh"));
    }

    scripts.push(PathBuf::from("/usr/local/bin/suspend-web-launch"));
    scripts
}

async fn run_attempt(
    command: &str,
    args: Vec<String>,
    label: String,
    attempts: &mut Vec<Value>,
) -> bool {
    let timeout_ms = 2_500_u64;
    let mut child = match Command::new(command).args(&args).spawn() {
        Ok(child) => child,
        Err(error) => {
            attempts.push(json!({
                "label": label,
                "command": command,
                "args": args,
                "success": false,
                "error": format!("spawn failed: {error}"),
            }));
            return false;
        }
    };

    let status_result = timeout(Duration::from_millis(timeout_ms), child.wait()).await;

    match status_result {
        Ok(Ok(status)) => {
            let success = status.success();
            attempts.push(json!({
                "label": label,
                "command": command,
                "args": args,
                "success": success,
                "exit_code": status.code(),
            }));
            success
        }
        Ok(Err(error)) => {
            attempts.push(json!({
                "label": label,
                "command": command,
                "args": args,
                "success": false,
                "error": format!("wait failed: {error}"),
            }));
            false
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            attempts.push(json!({
                "label": label,
                "command": command,
                "args": args,
                "success": false,
                "error": format!("timed out after {timeout_ms} ms"),
                "timed_out": true,
            }));
            false
        }
    }
}
