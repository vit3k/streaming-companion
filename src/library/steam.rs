use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::GameEntry;

pub fn read_all_steam_games() -> Vec<GameEntry> {
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

    // Deduplicate by canonical (real) path so that symlinked Steam roots
    // (e.g. ~/.steam/steam -> ~/.local/share/Steam) don't cause every game
    // to appear twice.
    let mut seen_canonical = HashSet::new();
    steamapps_dirs.retain(|dir| {
        let key = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        seen_canonical.insert(key)
    });

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
                if super::should_exclude_game_name(&game.name) {
                    continue;
                }
                games.push(game);
            }
        }
    }

    games
}

pub fn read_extra_steamapps_dirs_from_env() -> Vec<PathBuf> {
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

    // StateFlags bit 2 (value 4) means "fully installed". Manifests can exist
    // for games that are in your library but never installed (free weekends,
    // family shares, demos, etc.), so we skip those.
    let state_flags = extract_quoted_value(&content, "StateFlags")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    if state_flags & 4 == 0 {
        return None;
    }

    // Non-Steam launchers (Heroic, Lutris, etc.) sometimes plant fake
    // appmanifest files with the real Steam App ID so the game shows up in
    // Steam's UI, but the actual game files live elsewhere. Verify that the
    // installdir listed in the manifest actually exists under
    // steamapps/common/ — if it doesn't, this is a stub and we skip it.
    let installdir = extract_quoted_value(&content, "installdir").unwrap_or_default();
    if !installdir.is_empty() {
        let common = manifest_path
            .parent()
            .map(|steamapps| steamapps.join("common").join(&installdir));

        if !common.map(|p| p.is_dir()).unwrap_or(false) {
            return None;
        }
    }

    Some(GameEntry {
        id: appid,
        name,
        source: "steam",
    })
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
