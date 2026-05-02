use axum::{http::StatusCode, routing::{get, post}, Json, Router};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration, Instant};
use serde::{Deserialize, Serialize};

const BIND_ADDR: &str = "0.0.0.0:7878";
const STEAMGRIDDB_API_KEY: &str = "d9a9246d1f53fed39f924d3ab71f41e4";
const STEAMGRIDDB_API_BASE: &str = "https://www.steamgriddb.com/api/v2";
const POSTER_CACHE_VERSION: u8 = 2;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(BIND_ADDR).await?;
    println!("suspend-web listening on http://{BIND_ADDR}");

    axum::serve(listener, app()).await
}

fn app() -> Router {
    let games_routes = Router::new()
        .route("/", get(games_handler))
        .route("/launch", post(games_launch_handler))
        .route("/kill", post(games_kill_handler))
        .route("/:id/kill", post(game_kill_by_id_handler))
        .route("/:id/poster", get(get_game_poster_handler));
    
    Router::new()
        .route("/", get(root_handler))
        .route("/suspend", get(suspend_handler))
        .nest("/games", games_routes)
}

async fn root_handler() -> Json<Value> {
    Json(json!({
        "service": "suspend-web",
        "status": "ok",
        "message": "Service is running",
        "endpoints": ["/", "/suspend", "/games", "/games/launch", "/games/kill", "/games/:id/kill", "/games/:id/poster"]
    }))
}

async fn games_handler() -> (StatusCode, Json<Value>) {
    let games = read_all_games();
    let payload_games: Vec<Value> = games
        .iter()
        .map(|game| {
            let poster_url = if game.source == "steam" {
                Some(format!("/games/{}/poster", game.id))
            } else {
                None
            };

            json!({
                "id": game.id,
                "name": game.name,
                "source": game.source,
                "poster_url": poster_url,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!(payload_games)),
    )
}

async fn games_launch_handler(Json(payload): Json<Value>) -> (StatusCode, Json<Value>) {
    let requested_id = match payload.get("id").and_then(Value::as_str) {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "status": "error",
                    "error": "Missing required field: id"
                })),
            );
        }
    };

    if !requested_id.chars().all(|character| character.is_ascii_digit()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "error": "id must be a numeric Steam rungameid"
            })),
        );
    }

    let requested_source = payload
        .get("source")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let wait_for_start = payload
        .get("wait")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let wait_timeout_ms = payload
        .get("wait_timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(15_000)
        .clamp(1_000, 120_000);

    let known_games = read_all_games();
    let matched_game = known_games.into_iter().find(|game| {
        game.id == requested_id
            && requested_source
                .as_ref()
                .map(|source| game.source == source)
                .unwrap_or(true)
    });

    let game = match matched_game {
        Some(game) => game,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "status": "error",
                    "error": "Requested game id was not found in /games output"
                })),
            );
        }
    };

    match launch_game_with_steam(&game.id, wait_for_start, wait_timeout_ms).await {
        Ok(launch) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "message": "Launch requested",
                "game": {
                    "id": game.id,
                    "name": game.name,
                    "source": game.source
                },
                "launch": launch
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "status": "error",
                "error": format!("Failed to launch game: {error}")
            })),
        ),
    }
}

async fn games_kill_handler(Json(payload): Json<Value>) -> (StatusCode, Json<Value>) {
    let requested_id = match payload.get("id").and_then(Value::as_str) {
        Some(id) if !id.trim().is_empty() => id.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "status": "error",
                    "error": "Missing required field: id"
                })),
            );
        }
    };

    if !requested_id.chars().all(|character| character.is_ascii_digit()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "error": "id must be numeric"
            })),
        );
    }

    let requested_source = payload
        .get("source")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let force_kill_after_ms = payload
        .get("force_kill_after_ms")
        .and_then(Value::as_u64)
        .unwrap_or(2_000)
        .clamp(300, 15_000);

    kill_game_by_request(requested_id, requested_source, force_kill_after_ms).await
}

#[derive(Debug, Deserialize)]
struct KillByIdQuery {
    source: Option<String>,
    force_kill_after_ms: Option<u64>,
}

async fn game_kill_by_id_handler(
    axum::extract::Path(requested_id): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<KillByIdQuery>,
) -> (StatusCode, Json<Value>) {
    let requested_source = query
        .source
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let force_kill_after_ms = query
        .force_kill_after_ms
        .unwrap_or(2_000)
        .clamp(300, 15_000);

    kill_game_by_request(requested_id, requested_source, force_kill_after_ms).await
}

async fn kill_game_by_request(
    requested_id: String,
    requested_source: Option<String>,
    force_kill_after_ms: u64,
) -> (StatusCode, Json<Value>) {
    if !requested_id.chars().all(|character| character.is_ascii_digit()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "error": "id must be numeric"
            })),
        );
    }

    let known_games = read_all_games();
    let matched_game = known_games.into_iter().find(|game| {
        game.id == requested_id
            && requested_source
                .as_ref()
                .map(|source| game.source == source)
                .unwrap_or(true)
    });

    let game = match matched_game {
        Some(game) => game,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "status": "error",
                    "error": "Requested game id was not found in /games output"
                })),
            );
        }
    };

    let candidate_ids = launch_id_candidates(&game.id);
    let initial_pids = find_game_pids(&candidate_ids).await;

    if initial_pids.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "error",
                "error": "No running game process found for this id",
                "game": {
                    "id": game.id,
                    "name": game.name,
                    "source": game.source
                },
                "candidate_ids": candidate_ids
            })),
        );
    }

    let mut term_errors: Vec<String> = Vec::new();
    for pid in &initial_pids {
        if let Err(error) = send_signal(*pid, "-TERM").await {
            term_errors.push(format!("pid {pid}: {error}"));
        }
    }

    let deadline = Instant::now() + Duration::from_millis(force_kill_after_ms);
    let mut alive_after_term = initial_pids.clone();
    while Instant::now() < deadline {
        let mut still_alive = Vec::new();
        for pid in &alive_after_term {
            if process_exists(*pid).await {
                still_alive.push(*pid);
            }
        }

        alive_after_term = still_alive;
        if alive_after_term.is_empty() {
            break;
        }

        sleep(Duration::from_millis(150)).await;
    }

    let mut kill_errors: Vec<String> = Vec::new();
    let mut force_killed: Vec<i32> = Vec::new();
    for pid in &alive_after_term {
        match send_signal(*pid, "-KILL").await {
            Ok(()) => force_killed.push(*pid),
            Err(error) => kill_errors.push(format!("pid {pid}: {error}")),
        }
    }

    sleep(Duration::from_millis(100)).await;
    let mut still_running: Vec<i32> = Vec::new();
    for pid in &alive_after_term {
        if process_exists(*pid).await {
            still_running.push(*pid);
        }
    }

    let mut terminated: Vec<i32> = Vec::new();
    for pid in &initial_pids {
        if !still_running.contains(pid) {
            terminated.push(*pid);
        }
    }

    let status_code = if still_running.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (
        status_code,
        Json(json!({
            "status": if still_running.is_empty() { "ok" } else { "error" },
            "message": if still_running.is_empty() {
                "Kill request completed"
            } else {
                "Some processes are still running"
            },
            "game": {
                "id": game.id,
                "name": game.name,
                "source": game.source
            },
            "candidate_ids": candidate_ids,
            "force_kill_after_ms": force_kill_after_ms,
            "initial_pids": initial_pids,
            "terminated_pids": terminated,
            "force_killed_pids": force_killed,
            "still_running_pids": still_running,
            "errors": {
                "term": term_errors,
                "kill": kill_errors
            }
        })),
    )
}

async fn suspend_handler() -> (StatusCode, Json<Value>) {
    match suspend_system().await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "message": "System suspend requested"
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "status": "error",
                "error": format!("Failed to suspend: {error}")
            })),
        ),
    }
}

async fn suspend_system() -> Result<(), String> {
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

async fn launch_game_with_steam(id: &str, wait_for_start: bool, wait_timeout_ms: u64) -> Result<Value, String> {
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

fn launch_id_candidates(id: &str) -> Vec<String> {
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

fn shortcut_game_id_from_appid(appid: u32) -> u64 {
    ((appid as u64) << 32) | 0x0200_0000
}

fn extract_shortcut_appid_from_game_id(game_id: u64) -> Option<u32> {
    if (game_id & 0xFFFF_FFFF) != 0x0200_0000 {
        return None;
    }

    u32::try_from(game_id >> 32).ok()
}

fn launcher_script_candidates() -> Vec<PathBuf> {
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

async fn run_attempt(command: &str, args: Vec<String>, label: String, attempts: &mut Vec<Value>) -> bool {
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

async fn wait_for_appid_process(candidate_ids: &[String], timeout_ms: u64) -> bool {
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

async fn find_game_pids(candidate_ids: &[String]) -> Vec<i32> {
    let mut found = std::collections::BTreeSet::new();

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

async fn send_signal(pid: i32, signal: &str) -> Result<(), String> {
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

async fn process_exists(pid: i32) -> bool {
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

#[derive(Clone, Debug)]
struct GameEntry {
    id: String,
    name: String,
    source: &'static str,
}

fn read_all_games() -> Vec<GameEntry> {
    let mut games = Vec::new();
    games.extend(read_all_steam_games());
    games.extend(read_all_epic_games_from_shortcuts());

    games.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    games
}

fn read_all_steam_games() -> Vec<GameEntry> {
    let home = match env::var("HOME") {
        Ok(home) => home,
        Err(_) => return Vec::new(),
    };

    let mut roots = vec![
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];

    roots.retain(|root| root.is_dir());

    if roots.is_empty() {
        return Vec::new();
    }

    let mut steamapps_dirs: Vec<PathBuf> = Vec::new();
    for root in roots {
        let default_steamapps = root.join("steamapps");
        if default_steamapps.is_dir() {
            steamapps_dirs.push(default_steamapps.clone());
            let libraryfolders = default_steamapps.join("libraryfolders.vdf");
            steamapps_dirs.extend(read_library_steamapps_dirs(&libraryfolders));
        }
    }

    steamapps_dirs.extend(read_extra_steamapps_dirs_from_env());

    let mut seen_manifests = HashSet::new();
    let mut games: Vec<GameEntry> = Vec::new();

    for steamapps in steamapps_dirs {
        let entries = match fs::read_dir(&steamapps) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !is_appmanifest_path(&path) {
                continue;
            }

            if !seen_manifests.insert(path.clone()) {
                continue;
            }

            if let Some(game) = parse_steam_appmanifest(&path) {
                if should_exclude_game_name(&game.name) {
                    continue;
                }
                games.push(game);
            }
        }
    }

    games
}

fn read_extra_steamapps_dirs_from_env() -> Vec<PathBuf> {
    let raw = match env::var("SUSPEND_WEB_STEAMAPPS_DIRS") {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let mut dirs = Vec::new();
    let separator = if raw.contains(';') { ';' } else { ':' };

    for token in raw.split(separator) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut path = PathBuf::from(trimmed);
        if path.ends_with("steamapps") {
            if path.is_dir() {
                dirs.push(path);
            }
            continue;
        }

        path = path.join("steamapps");
        if path.is_dir() {
            dirs.push(path);
        }
    }

    dirs
}

fn read_all_epic_games_from_shortcuts() -> Vec<GameEntry> {
    let mut games = Vec::new();
    let mut seen = HashSet::new();

    for shortcuts_path in find_shortcuts_vdf_files() {
        let bytes = match fs::read(shortcuts_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        for entry in parse_shortcuts_vdf_entries(&bytes) {
            if !is_epic_shortcut(&entry)
                || should_exclude_game_name(&entry.appname)
                || should_exclude_shortcut_entry(&entry)
            {
                continue;
            }

            let game = GameEntry {
                id: shortcut_game_id_from_appid(entry.appid).to_string(),
                name: entry.appname,
                source: "epic",
            };

            let dedupe_key = format!("{}:{}", game.source, game.id);
            if seen.insert(dedupe_key) {
                games.push(game);
            }
        }
    }

    games
}

fn read_library_steamapps_dirs(libraryfolders_path: &Path) -> Vec<PathBuf> {
    let content = match fs::read_to_string(libraryfolders_path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    let mut search_from = 0usize;

    while let Some(relative_pos) = content[search_from..].find("\"path\"") {
        let key_pos = search_from + relative_pos;
        let after_key = &content[key_pos + "\"path\"".len()..];

        let first_quote = match after_key.find('"') {
            Some(index) => index,
            None => break,
        };

        let remaining = &after_key[first_quote + 1..];
        let second_quote = match remaining.find('"') {
            Some(index) => index,
            None => break,
        };

        let raw_path = &remaining[..second_quote];
        let normalized = raw_path.replace("\\\\", "\\");
        let steamapps = PathBuf::from(normalized).join("steamapps");
        if steamapps.is_dir() && seen.insert(steamapps.clone()) {
            dirs.push(steamapps);
        }

        search_from = key_pos + "\"path\"".len() + first_quote + 1 + second_quote + 1;
        if search_from >= content.len() {
            break;
        }
    }

    dirs
}

fn is_appmanifest_path(path: &Path) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.starts_with("appmanifest_") && name.ends_with(".acf"),
        None => false,
    }
}

fn parse_steam_appmanifest(manifest_path: &Path) -> Option<GameEntry> {
    let content = fs::read_to_string(manifest_path).ok()?;
    let appid = extract_quoted_value(&content, "appid")?;
    let name = extract_quoted_value(&content, "name")?;

    Some(GameEntry {
        id: appid,
        name,
        source: "steam",
    })
}

fn should_exclude_game_name(name: &str) -> bool {
    let name = name.to_lowercase();

    name.contains("steamworks common redistributables")
        || name.contains("steam linux runtime")
        || name.contains("proton")
}

fn find_shortcuts_vdf_files() -> Vec<PathBuf> {
    let home = match env::var("HOME") {
        Ok(home) => home,
        Err(_) => return Vec::new(),
    };

    let roots = [
        PathBuf::from(&home).join(".local/share/Steam"),
        PathBuf::from(&home).join(".steam/steam"),
        PathBuf::from(&home).join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];

    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for root in roots {
        let userdata = root.join("userdata");
        let users = match fs::read_dir(userdata) {
            Ok(users) => users,
            Err(_) => continue,
        };

        for user in users.flatten() {
            let path = user.path().join("config/shortcuts.vdf");
            if path.is_file() && seen.insert(path.clone()) {
                files.push(path);
            }
        }
    }

    files
}

#[derive(Debug)]
struct ShortcutEntry {
    appname: String,
    appid: u32,
    exe: String,
    start_dir: String,
    launch_options: String,
}

fn parse_shortcuts_vdf_entries(bytes: &[u8]) -> Vec<ShortcutEntry> {
    let mut entries = Vec::new();
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        if parse_vdf_object(bytes, &mut cursor, None, &mut entries).is_none() {
            break;
        }

        if cursor < bytes.len() && bytes[cursor] == 0x08 {
            cursor += 1;
        }
    }

    entries
}

fn parse_vdf_object(
    bytes: &[u8],
    cursor: &mut usize,
    object_key: Option<&str>,
    entries: &mut Vec<ShortcutEntry>,
) -> Option<()> {
    let mut appname: Option<String> = None;
    let mut appid: Option<u32> = None;
    let mut exe: Option<String> = None;
    let mut start_dir: Option<String> = None;
    let mut launch_options: Option<String> = None;

    loop {
        let field_type = *bytes.get(*cursor)?;
        *cursor += 1;

        if field_type == 0x08 {
            break;
        }

        let key = read_cstring(bytes, cursor)?;
        let key_lc = key.to_ascii_lowercase();

        match field_type {
            0x00 => {
                parse_vdf_object(bytes, cursor, Some(&key), entries)?;
            }
            0x01 => {
                let value = read_cstring(bytes, cursor)?;
                if key_lc == "appname" {
                    appname = Some(value);
                } else if key_lc == "exe" {
                    exe = Some(value);
                } else if key_lc == "startdir" {
                    start_dir = Some(value);
                } else if key_lc == "launchoptions" {
                    launch_options = Some(value);
                }
            }
            0x02 => {
                let value = read_u32_le(bytes, cursor)?;
                if key_lc == "appid" {
                    appid = Some(value);
                }
            }
            0x03 | 0x06 => {
                *cursor = cursor.checked_add(4)?;
                if *cursor > bytes.len() {
                    return None;
                }
            }
            0x07 | 0x0a => {
                *cursor = cursor.checked_add(8)?;
                if *cursor > bytes.len() {
                    return None;
                }
            }
            _ => {
                return None;
            }
        }
    }

    if object_key
        .map(|key| key.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
    {
        if let (Some(appname), Some(appid)) = (appname, appid) {
            entries.push(ShortcutEntry {
                appname,
                appid,
                exe: exe.unwrap_or_default(),
                start_dir: start_dir.unwrap_or_default(),
                launch_options: launch_options.unwrap_or_default(),
            });
        }
    }

    Some(())
}

fn read_cstring(bytes: &[u8], cursor: &mut usize) -> Option<String> {
    let start = *cursor;
    let mut end = start;

    while end < bytes.len() && bytes[end] != 0 {
        end += 1;
    }

    if end >= bytes.len() {
        return None;
    }

    *cursor = end + 1;
    Some(String::from_utf8_lossy(&bytes[start..end]).to_string())
}

fn read_u32_le(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let end = cursor.checked_add(4)?;
    let raw = bytes.get(*cursor..end)?;
    let value = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    *cursor = end;
    Some(value)
}

fn is_epic_shortcut(entry: &ShortcutEntry) -> bool {
    let combined = format!(
        "{} {} {} {}",
        entry.appname, entry.exe, entry.start_dir, entry.launch_options
    )
    .to_lowercase();

    combined.contains("heroic")
        || combined.contains("legendary")
        || combined.contains("epic")
        || combined.contains("com.heroicgameslauncher.hgl")
}

fn should_exclude_shortcut_entry(entry: &ShortcutEntry) -> bool {
    let combined = format!(
        "{} {} {} {}",
        entry.appname, entry.exe, entry.start_dir, entry.launch_options
    )
    .to_lowercase();

    let is_obvious_launcher = combined.contains("heroic games launcher")
        || combined.contains("legendary launcher")
        || combined.contains("epic games launcher")
        || combined.contains("lutris")
        || combined.contains("bottles");

    if is_obvious_launcher {
        return true;
    }

    false
}

fn extract_quoted_value(content: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let key_pos = content.find(&pattern)?;
    let after_key = &content[key_pos + pattern.len()..];

    let first_quote = after_key.find('"')?;
    let remaining = &after_key[first_quote + 1..];
    let second_quote = remaining.find('"')?;

    Some(remaining[..second_quote].to_string())
}

// ============================================================================
// SteamGridDB Integration
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PosterInfo {
    local_path: String,
    fetched_at: u64,
    #[serde(default)]
    cache_version: u8,
}

#[derive(Debug, Deserialize)]
struct SteamGridDBResponse {
    success: bool,
    data: Option<Vec<SteamGridDBImage>>,
}

#[derive(Debug, Deserialize, Clone)]
struct SteamGridDBImage {
    #[allow(dead_code)]
    id: u64,
    url: String,
    #[allow(dead_code)]
    thumb: String,
    #[allow(dead_code)]
    #[serde(default)]
    style: String,
}

fn get_cache_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(&home).join(".cache/suspend-web")
}

fn get_poster_cache_path(app_id: &str) -> PathBuf {
    get_cache_dir().join(format!("poster_{}.json", app_id))
}

async fn fetch_poster_from_steamgriddb(steam_app_id: &str) -> Option<PosterInfo> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/grids/steam/{}?dimensions=600x900&limit=1",
        STEAMGRIDDB_API_BASE, steam_app_id
    );

    let response = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", STEAMGRIDDB_API_KEY))
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            eprintln!("Failed to fetch from SteamGridDB for steam app {}: {}", steam_app_id, e);
            return None;
        }
    };

    let text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to read response body for steam app {}: {}", steam_app_id, e);
            return None;
        }
    };

    let data: SteamGridDBResponse = match serde_json::from_str(&text) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to parse SteamGridDB response for steam app {}: {}", steam_app_id, e);
            eprintln!("Raw response: {}", text);
            return None;
        }
    };

    if !data.success {
        eprintln!("SteamGridDB API error for steam app {}: {:?}", steam_app_id, data);
        return None;
    }

    let images = data.data?;
    if images.is_empty() {
        eprintln!("No posters found in SteamGridDB for steam app {}", steam_app_id);
        return None;
    }

    let best_image = images.first()?;

    // Download the poster image
    let poster_client = reqwest::Client::new();
    let image_data = match poster_client
        .get(&best_image.url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
    {
        Ok(res) => match res.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => {
                eprintln!("Failed to read poster bytes for steam app {}: {}", steam_app_id, e);
                return None;
            }
        },
        Err(e) => {
            eprintln!("Failed to download poster for steam app {}: {}", steam_app_id, e);
            return None;
        }
    };

    // Ensure posters directory exists
    let posters_dir = get_cache_dir().join("posters");
    if fs::create_dir_all(&posters_dir).is_err() {
        eprintln!("Failed to create posters directory");
        return None;
    }

    // Save poster locally
    let poster_filename = format!("{}.png", steam_app_id);
    let poster_path = posters_dir.join(&poster_filename);
    if fs::write(&poster_path, &image_data).is_err() {
        eprintln!("Failed to write poster file for steam app {}", steam_app_id);
        return None;
    }

    let poster_info = PosterInfo {
        local_path: format!("posters/{}", poster_filename),
        fetched_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        cache_version: POSTER_CACHE_VERSION,
    };

    // Cache the result
    if let Ok(cache_dir) = fs::metadata(get_cache_dir()) {
        if cache_dir.is_dir() {
            let cache_path = get_poster_cache_path(steam_app_id);
            if let Ok(json) = serde_json::to_string(&poster_info) {
                let _ = fs::write(&cache_path, json);
            }
        }
    } else {
        let cache_dir = get_cache_dir();
        if fs::create_dir_all(&cache_dir).is_ok() {
            let cache_path = get_poster_cache_path(steam_app_id);
            if let Ok(json) = serde_json::to_string(&poster_info) {
                let _ = fs::write(&cache_path, json);
            }
        }
    }

    Some(poster_info)
}

fn load_poster_from_cache(app_id: &str) -> Option<PosterInfo> {
    let cache_path = get_poster_cache_path(app_id);
    let json_str = fs::read_to_string(&cache_path).ok()?;
    serde_json::from_str(&json_str).ok()
}

async fn get_game_poster(steam_app_id: &str) -> Option<PosterInfo> {
    // Try cache first
    if let Some(poster) = load_poster_from_cache(steam_app_id) {
        let poster_path = get_cache_dir().join(&poster.local_path);
        // Consider cache valid for 7 days
        let age = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            - poster.fetched_at;
        
        if age < 7 * 24 * 60 * 60
            && poster_path.is_file()
            && poster.cache_version == POSTER_CACHE_VERSION
        {
            return Some(poster);
        }
    }

    // Fetch fresh
    fetch_poster_from_steamgriddb(steam_app_id).await
}

async fn get_game_poster_handler(
    axum::extract::Path(game_id): axum::extract::Path<String>,
) -> Result<(axum::http::HeaderMap, Vec<u8>), (StatusCode, String)> {
    // Find the game to verify it exists and is a Steam game
    let games = read_all_games();
    let game = match games.iter().find(|g| g.id == game_id && g.source == "steam") {
        Some(g) => g,
        None => return Err((StatusCode::NOT_FOUND, "Game not found or not a Steam game".to_string())),
    };

    match get_game_poster(&game.id).await {
        Some(poster) => {
            let poster_path = get_cache_dir().join(&poster.local_path);
            match fs::read(&poster_path) {
                Ok(data) => {
                    let mut headers = axum::http::HeaderMap::new();
                    headers.insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("image/png"),
                    );
                    Ok((headers, data))
                }
                Err(_) => Err((StatusCode::INTERNAL_SERVER_ERROR, "Poster file missing from cache".to_string())),
            }
        }
        None => Err((StatusCode::NOT_FOUND, "No poster found for this game".to_string())),
    }
}


