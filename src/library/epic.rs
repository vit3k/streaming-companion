use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use super::vdf::{find_shortcuts_vdf_files, parse_shortcuts_vdf_entries, ShortcutEntry};
use super::GameEntry;
use crate::process::shortcut_game_id_from_appid;

pub fn read_all_shortcut_games() -> Vec<GameEntry> {
    let mut games = Vec::new();
    let mut seen = HashSet::new();

    for shortcuts_path in find_shortcuts_vdf_files() {
        let bytes = match fs::read(shortcuts_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        for entry in parse_shortcuts_vdf_entries(&bytes) {
            if super::should_exclude_game_name(&entry.appname)
                || should_exclude_shortcut_entry(&entry)
            {
                continue;
            }

            let Some(source) = detect_shortcut_source(&entry) else {
                continue;
            };

            let game = GameEntry {
                id: shortcut_game_id_from_appid(entry.appid).to_string(),
                name: entry.appname,
                source,
            };

            let dedupe_key = format!("{}:{}", game.source, game.id);
            if seen.insert(dedupe_key) {
                games.push(game);
            }
        }
    }

    games
}

/// Determines the source of a Steam shortcut entry.
/// Returns `None` if the entry doesn't belong to a recognised non-Steam launcher.
fn detect_shortcut_source(entry: &ShortcutEntry) -> Option<&'static str> {
    let opts_lc = entry.launch_options.to_lowercase();
    let combined_lc = format!(
        "{} {} {} {}",
        entry.appname, entry.exe, entry.start_dir, entry.launch_options
    )
    .to_lowercase();

    // ── Heroic Games Launcher ────────────────────────────────────────────────
    // Heroic manages Epic, GOG, and arbitrary sideloaded games. We
    // distinguish them by the heroic:// URI embedded in the launch options.
    let is_heroic =
        combined_lc.contains("heroic") || combined_lc.contains("com.heroicgameslauncher.hgl");

    if is_heroic {
        if opts_lc.contains("heroic://launch/gog/") {
            return Some("gog");
        }

        if opts_lc.contains("heroic://launch/legendary/") {
            return Some("epic");
        }

        if opts_lc.contains("heroic://launch/sideload/") {
            // Extract the app identifier from the URI so we can look up
            // Heroic's own config and determine the real launcher.
            let app_id = opts_lc
                .split("heroic://launch/sideload/")
                .nth(1)
                .and_then(|tail| tail.split(|c: char| c.is_whitespace() || c == '"').next())
                .unwrap_or("")
                .to_string();

            return Some(detect_sideload_source(&app_id));
        }

        // Heroic shortcut without a typed URI — fall back based on other clues.
        if combined_lc.contains("epic") || combined_lc.contains("legendary") {
            return Some("epic");
        }

        // Default for unrecognised Heroic shortcuts.
        return Some("epic");
    }

    // ── Direct Battle.net shortcuts ──────────────────────────────────────────
    // Some users add Battle.net (or individual Blizzard games) to Steam
    // themselves, without going through Heroic.
    if combined_lc.contains("battle.net")
        || combined_lc.contains("battlenet")
        || combined_lc.contains("blizzard")
    {
        return Some("battlenet");
    }

    // ── Legendary CLI (standalone, without Heroic UI) ────────────────────────
    if combined_lc.contains("legendary") {
        return Some("epic");
    }

    None
}

/// For a Heroic sideloaded game, read its GamesConfig JSON to figure out
/// which launcher it actually uses (Battle.net, itch.io, etc.).
/// Falls back to `"sideload"` when it can't be determined.
fn detect_sideload_source(app_id: &str) -> &'static str {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => return "sideload",
    };

    // Heroic stores per-game config under GamesConfig/ for both the native
    // install and the Flatpak.
    let config_dirs = [
        PathBuf::from(&home).join(".config/heroic/GamesConfig"),
        PathBuf::from(&home).join(".var/app/com.heroicgameslauncher.hgl/config/heroic/GamesConfig"),
    ];

    for dir in &config_dirs {
        let config_file = dir.join(format!("{app_id}.json"));
        if let Ok(content) = fs::read_to_string(&config_file) {
            let lc = content.to_lowercase();
            if lc.contains("battle.net")
                || lc.contains("battlenet")
                || lc.contains("blizzard")
                || lc.contains("battle net")
            {
                return "battlenet";
            }
        }
    }

    "sideload"
}

fn should_exclude_shortcut_entry(entry: &ShortcutEntry) -> bool {
    let combined = format!(
        "{} {} {} {}",
        entry.appname, entry.exe, entry.start_dir, entry.launch_options
    )
    .to_lowercase();

    combined.contains("heroic games launcher")
        || combined.contains("legendary launcher")
        || combined.contains("epic games launcher")
        || combined.contains("battle.net launcher")
        || combined.contains("lutris")
        || combined.contains("bottles")
}
