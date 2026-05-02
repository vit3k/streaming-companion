use std::collections::{BTreeSet, HashMap, VecDeque};
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
    // Read /proc/<pid>/status directly so we can treat zombie processes
    // (State: Z) as dead. kill -0 returns 0 for zombies, which caused
    // false "still running" reports after SIGKILL.
    let path = format!("/proc/{}/status", pid);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("State:") {
                    // 'Z' = zombie: the process is dead but not yet reaped.
                    return !rest.trim_start().starts_with('Z');
                }
            }
            true
        }
        Err(_) => false,
    }
}

/// Returns all transitive descendant PIDs of `root_pid` by taking a snapshot
/// of `/proc` and doing a BFS over the parent→child relationships.
///
/// Reads `/proc` synchronously; `/proc` is an in-memory virtual filesystem so
/// this is effectively instantaneous even with hundreds of processes.
pub fn find_process_descendants(root_pid: i32) -> Vec<i32> {
    // Build a ppid → [children] map from the current /proc snapshot.
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();

    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let pid: i32 = match fname.to_string_lossy().parse() {
            Ok(p) => p,
            Err(_) => continue, // skip non-numeric /proc entries
        };

        let status_path = entry.path().join("status");
        let content = match std::fs::read_to_string(&status_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("PPid:") {
                if let Ok(ppid) = rest.trim().parse::<i32>() {
                    children.entry(ppid).or_default().push(pid);
                }
                break;
            }
        }
    }

    // BFS from root_pid to collect every descendant.
    let mut result = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(root_pid);

    while let Some(pid) = queue.pop_front() {
        if let Some(kids) = children.get(&pid) {
            for &child in kids {
                result.push(child);
                queue.push_back(child);
            }
        }
    }

    result
}
