use std::collections::HashSet;
use std::fs;

use super::GameEntry;
use super::vdf::{find_shortcuts_vdf_files, parse_shortcuts_vdf_entries, ShortcutEntry};
use crate::process::shortcut_game_id_from_appid;

pub fn read_all_epic_games_from_shortcuts() -> Vec<GameEntry> {
    let mut games = Vec::new();
    let mut seen = HashSet::new();

    for shortcuts_path in find_shortcuts_vdf_files() {
        let bytes = match fs::read(shortcuts_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        for entry in parse_shortcuts_vdf_entries(&bytes) {
            if !is_epic_shortcut(&entry)
                || super::should_exclude_game_name(&entry.appname)
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

    combined.contains("heroic games launcher")
        || combined.contains("legendary launcher")
        || combined.contains("epic games launcher")
        || combined.contains("lutris")
        || combined.contains("bottles")
}
