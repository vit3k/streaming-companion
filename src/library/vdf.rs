use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Finds all `shortcuts.vdf` files across the known Steam root directories.
pub fn find_shortcuts_vdf_files() -> Vec<PathBuf> {
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
pub struct ShortcutEntry {
    pub appname: String,
    pub appid: u32,
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
}

pub fn parse_shortcuts_vdf_entries(bytes: &[u8]) -> Vec<ShortcutEntry> {
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
