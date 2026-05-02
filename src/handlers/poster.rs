use std::fs;

use axum::http::StatusCode;

use crate::library::read_all_games;
use crate::poster::{get_cache_dir, get_game_poster};

pub async fn get_game_poster_handler(
    axum::extract::Path(game_id): axum::extract::Path<String>,
) -> Result<(axum::http::HeaderMap, Vec<u8>), (StatusCode, String)> {
    let games = read_all_games();
    let game = match games.iter().find(|g| g.id == game_id && g.source == "steam") {
        Some(g) => g,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                "Game not found or not a Steam game".to_string(),
            ))
        }
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
                Err(_) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Poster file missing from cache".to_string(),
                )),
            }
        }
        None => Err((StatusCode::NOT_FOUND, "No poster found for this game".to_string())),
    }
}
