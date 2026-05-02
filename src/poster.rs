use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::time::Duration;

const STEAMGRIDDB_API_BASE: &str = "https://www.steamgriddb.com/api/v2";
const STEAMGRIDDB_API_KEY_ENV: &str = "STEAMGRIDDB_API_KEY";
pub const POSTER_CACHE_VERSION: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PosterInfo {
    pub local_path: String,
    pub fetched_at: u64,
    #[serde(default)]
    pub cache_version: u8,
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

pub fn get_cache_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(&home).join(".cache/suspend-web")
}

pub fn get_poster_cache_path(app_id: &str) -> PathBuf {
    get_cache_dir().join(format!("poster_{}.json", app_id))
}

pub async fn fetch_poster_from_steamgriddb(steam_app_id: &str) -> Option<PosterInfo> {
    let api_key = match env::var(STEAMGRIDDB_API_KEY_ENV) {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!(
                "SteamGridDB API key not set. \
                 Set the {} environment variable to enable poster fetching.",
                STEAMGRIDDB_API_KEY_ENV
            );
            return None;
        }
    };

    let client = reqwest::Client::new();
    let url = format!(
        "{}/grids/steam/{}?dimensions=600x900&limit=1",
        STEAMGRIDDB_API_BASE, steam_app_id
    );

    let response = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            eprintln!(
                "Failed to fetch from SteamGridDB for steam app {}: {}",
                steam_app_id, e
            );
            return None;
        }
    };

    let text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Failed to read response body for steam app {}: {}",
                steam_app_id, e
            );
            return None;
        }
    };

    let data: SteamGridDBResponse = match serde_json::from_str(&text) {
        Ok(data) => data,
        Err(e) => {
            eprintln!(
                "Failed to parse SteamGridDB response for steam app {}: {}",
                steam_app_id, e
            );
            eprintln!("Raw response: {}", text);
            return None;
        }
    };

    if !data.success {
        eprintln!(
            "SteamGridDB API error for steam app {}: {:?}",
            steam_app_id, data
        );
        return None;
    }

    let images = data.data?;
    if images.is_empty() {
        eprintln!(
            "No posters found in SteamGridDB for steam app {}",
            steam_app_id
        );
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
                eprintln!(
                    "Failed to read poster bytes for steam app {}: {}",
                    steam_app_id, e
                );
                return None;
            }
        },
        Err(e) => {
            eprintln!(
                "Failed to download poster for steam app {}: {}",
                steam_app_id, e
            );
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

pub fn load_poster_from_cache(app_id: &str) -> Option<PosterInfo> {
    let cache_path = get_poster_cache_path(app_id);
    let json_str = fs::read_to_string(&cache_path).ok()?;
    serde_json::from_str(&json_str).ok()
}

pub async fn get_game_poster(steam_app_id: &str) -> Option<PosterInfo> {
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
