use axum::{http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};

use crate::launcher::launch_game_with_steam;
use crate::library::{read_all_games, GameEntry};
use crate::process::{find_game_pids, find_process_descendants, launch_id_candidates, process_exists, send_signal};

// ── List all games ────────────────────────────────────────────────────────────

pub async fn games_handler() -> (StatusCode, Json<Value>) {
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

    (StatusCode::OK, Json(json!(payload_games)))
}

// ── Launch ────────────────────────────────────────────────────────────────────

pub async fn games_launch_handler(Json(payload): Json<Value>) -> (StatusCode, Json<Value>) {
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

// ── Kill by JSON body ─────────────────────────────────────────────────────────

pub async fn games_kill_handler(Json(payload): Json<Value>) -> (StatusCode, Json<Value>) {
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

// ── Kill by path /:id/kill ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct KillByIdQuery {
    pub source: Option<String>,
    pub force_kill_after_ms: Option<u64>,
}

pub async fn game_kill_by_id_handler(
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

// ── Detect running game ───────────────────────────────────────────────────────

/// `GET /games/running` — returns every known game that currently has a
/// matching live process.
pub async fn games_running_handler() -> (StatusCode, Json<Value>) {
    let running = detect_running_games().await;

    let payload: Vec<Value> = running
        .into_iter()
        .map(|(game, candidate_ids, pids)| {
            json!({
                "id": game.id,
                "name": game.name,
                "source": game.source,
                "pids": pids,
                "candidate_ids": candidate_ids,
            })
        })
        .collect();

    (StatusCode::OK, Json(json!(payload)))
}

// ── Stop running game ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StopQuery {
    pub force_kill_after_ms: Option<u64>,
}

/// `POST /games/running/stop` — auto-detects whichever game is currently
/// running and stops it. No game ID required.
pub async fn games_running_stop_handler(
    axum::extract::Query(query): axum::extract::Query<StopQuery>,
) -> (StatusCode, Json<Value>) {
    let force_kill_after_ms = query
        .force_kill_after_ms
        .unwrap_or(2_000)
        .clamp(300, 15_000);

    let running = detect_running_games().await;

    if running.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "error",
                "error": "No running game found"
            })),
        );
    }

    let mut results: Vec<Value> = Vec::new();

    for (game, candidate_ids, initial_pids) in &running {
        let result = terminate_pids(initial_pids, force_kill_after_ms).await;
        results.push(json!({
            "game": {
                "id": game.id,
                "name": game.name,
                "source": game.source,
            },
            "candidate_ids": candidate_ids,
            "force_kill_after_ms": force_kill_after_ms,
            "initial_pids": initial_pids,
            "terminated_pids": result.terminated,
            "force_killed_pids": result.force_killed,
            "still_running_pids": result.still_running,
            "errors": {
                "term": result.term_errors,
                "kill": result.kill_errors,
            }
        }));
    }

    let all_stopped = results.iter().all(|r| {
        r["still_running_pids"]
            .as_array()
            .map(|pids| pids.is_empty())
            .unwrap_or(true)
    });

    let status_code = if all_stopped {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (
        status_code,
        Json(json!({
            "status": if all_stopped { "ok" } else { "error" },
            "message": if all_stopped {
                "All running games stopped"
            } else {
                "Some processes are still running"
            },
            "stopped": results,
        })),
    )
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Scans all known games and returns those that currently have live processes.
async fn detect_running_games() -> Vec<(GameEntry, Vec<String>, Vec<i32>)> {
    let mut running = Vec::new();

    for game in read_all_games() {
        let candidate_ids = launch_id_candidates(&game.id);
        let pids = find_game_pids(&candidate_ids).await;
        if !pids.is_empty() {
            running.push((game, candidate_ids, pids));
        }
    }

    running
}

/// Resolves a game by ID + optional source filter, then kills it.
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

    let result = terminate_pids(&initial_pids, force_kill_after_ms).await;

    let status_code = if result.still_running.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (
        status_code,
        Json(json!({
            "status": if result.still_running.is_empty() { "ok" } else { "error" },
            "message": if result.still_running.is_empty() {
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
            "terminated_pids": result.terminated,
            "force_killed_pids": result.force_killed,
            "still_running_pids": result.still_running,
            "errors": {
                "term": result.term_errors,
                "kill": result.kill_errors
            }
        })),
    )
}

/// Sends SIGTERM to every PID, waits up to `force_kill_after_ms` for graceful
/// exit, then SIGKILLs any survivors.
struct TerminateResult {
    terminated: Vec<i32>,
    force_killed: Vec<i32>,
    still_running: Vec<i32>,
    term_errors: Vec<String>,
    kill_errors: Vec<String>,
}

async fn terminate_pids(initial_pids: &[i32], force_kill_after_ms: u64) -> TerminateResult {
    // Expand to the full process tree so we kill every descendant (Proton
    // layers, Wine, the actual game binary, etc.) and not just the process
    // that happened to have AppId= in its command line.
    let mut all_pids: Vec<i32> = initial_pids.to_vec();
    for &pid in initial_pids {
        for descendant in find_process_descendants(pid) {
            if !all_pids.contains(&descendant) {
                all_pids.push(descendant);
            }
        }
    }

    // Phase 1: SIGTERM
    let mut term_errors: Vec<String> = Vec::new();
    for pid in &all_pids {
        if let Err(error) = send_signal(*pid, "-TERM").await {
            term_errors.push(format!("pid {pid}: {error}"));
        }
    }

    // Phase 2: wait for graceful exit
    let deadline = Instant::now() + Duration::from_millis(force_kill_after_ms);
    let mut alive_after_term = all_pids.clone();
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

    // Phase 3: SIGKILL survivors
    let mut kill_errors: Vec<String> = Vec::new();
    let mut force_killed: Vec<i32> = Vec::new();
    for pid in &alive_after_term {
        match send_signal(*pid, "-KILL").await {
            Ok(()) => force_killed.push(*pid),
            Err(error) => kill_errors.push(format!("pid {pid}: {error}")),
        }
    }

    // Phase 4: poll until SIGKILL'd processes are fully reaped (up to 3 s).
    // A fixed sleep is not enough — large games can take a moment to disappear
    // from /proc even after SIGKILL is delivered.
    let kill_deadline = Instant::now() + Duration::from_millis(3_000);
    let mut still_running = force_killed.clone();
    while !still_running.is_empty() && Instant::now() < kill_deadline {
        sleep(Duration::from_millis(150)).await;
        let mut alive = Vec::new();
        for pid in &still_running {
            if process_exists(*pid).await {
                alive.push(*pid);
            }
        }
        still_running = alive;
    }

    let terminated = initial_pids
        .iter()
        .filter(|pid| !still_running.contains(pid))
        .copied()
        .collect();

    TerminateResult {
        terminated,
        force_killed,
        still_running,
        term_errors,
        kill_errors,
    }
}
