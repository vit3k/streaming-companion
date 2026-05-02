use std::collections::BTreeSet;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

pub fn launch_id_candidates(id: &str) -> Vec<String> {
    let mut candidates = vec![id.to_string()];

    if let Ok(raw) = id.parse::<u64>() {
        if let Some(shortcut_appid) = extract_shortcut_appid_from_game_id(raw) {
            let appid = shortcut_appid.to_string();
            if !candidates.contains(&appid) {
                candidates.push(appid);
            }

            let signed = (shortcut_appid as i32).to_string();
            if !candidates.contains(&signed) {
                candidates.push(signed);
            }
        }

        if let Ok(unsigned32) = u32::try_from(raw) {
            let signed = (unsigned32 as i32).to_string();
            if !candidates.contains(&signed) {
                candidates.push(signed);
            }
        }
    }

    candidates
}

pub fn shortcut_game_id_from_appid(appid: u32) -> u64 {
    ((appid as u64) << 32) | 0x0200_0000
}

pub fn extract_shortcut_appid_from_game_id(game_id: u64) -> Option<u32> {
    if (game_id & 0xFFFF_FFFF) != 0x0200_0000 {
        return None;
    }

    u32::try_from(game_id >> 32).ok()
}

pub async fn wait_for_appid_process(candidate_ids: &[String], timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    while Instant::now() < deadline {
        for candidate in candidate_ids {
            let pattern = format!("AppId={candidate}");
            if let Ok(output) = Command::new("pgrep")
                .arg("-f")
                .arg("-i")
                .arg(&pattern)
                .output()
                .await
            {
                if output.status.success() && !output.stdout.is_empty() {
                    return true;
                }
            }
        }

        sleep(Duration::from_millis(400)).await;
    }

    false
}

pub async fn find_game_pids(candidate_ids: &[String]) -> Vec<i32> {
    let mut found = BTreeSet::new();

    for candidate in candidate_ids {
        let patterns = [
            format!("AppId={candidate}"),
            format!("SteamAppId={candidate}"),
            format!("steam_appid={candidate}"),
        ];

        for pattern in patterns {
            let output = match Command::new("pgrep")
                .arg("-f")
                .arg("-i")
                .arg(&pattern)
                .output()
                .await
            {
                Ok(output) => output,
                Err(_) => continue,
            };

            if !output.status.success() {
                continue;
            }

            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Ok(pid) = line.trim().parse::<i32>() {
                    found.insert(pid);
                }
            }
        }
    }

    found.into_iter().collect()
}

pub async fn send_signal(pid: i32, signal: &str) -> Result<(), String> {
    let status = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|error| format!("kill command failed: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("kill exited with status {status}"))
    }
}

pub async fn process_exists(pid: i32) -> bool {
    match Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .await
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}
