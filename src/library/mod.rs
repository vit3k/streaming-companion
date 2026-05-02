mod epic;
mod steam;
pub mod vdf;

pub use epic::read_all_epic_games_from_shortcuts;
pub use steam::read_all_steam_games;

#[derive(Clone, Debug)]
pub struct GameEntry {
    pub id: String,
    pub name: String,
    pub source: &'static str,
}

pub fn read_all_games() -> Vec<GameEntry> {
    let mut games = Vec::new();
    games.extend(read_all_steam_games());
    games.extend(read_all_epic_games_from_shortcuts());

    games.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    games
}

/// Returns true for games that are infrastructure / runtimes rather than
/// actual playable titles (Proton, Steam Linux Runtime, etc.).
pub fn should_exclude_game_name(name: &str) -> bool {
    let name = name.to_lowercase();

    name.contains("steamworks common redistributables")
        || name.contains("steam linux runtime")
        || name.contains("proton")
}
