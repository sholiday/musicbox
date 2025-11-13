use crate::controller::{CardUid, CardUidParseError, Library, Track};
use chrono::{Local, NaiveTime};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use std::path::Path;
use toml_edit::{DocumentMut, table, value};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    ParseToml(#[from] toml::de::Error),
    #[error("invalid card uid: {0}")]
    CardUid(#[from] CardUidParseError),
    #[error("duplicate mapping for card {0:?}")]
    DuplicateCard(CardUid),
    #[error("invalid volume setting {field}: {reason}")]
    InvalidVolume { field: &'static str, reason: String },
    #[error("invalid quiet hours entry at index {index}: {reason}")]
    InvalidQuietHours { index: usize, reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigEditError {
    #[error("failed to read config {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write config {path:?}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml_edit::TomlError),
    #[error("config missing [cards] table")]
    MissingCards,
    #[error("card {0:?} already mapped in config")]
    Duplicate(CardUid),
}

/// Represents the configuration for the music box.
#[derive(Debug, Clone)]
pub struct MusicBoxConfig {
    music_dir: PathBuf,
    cards: HashMap<CardUid, PathBuf>,
    volume: VolumeSettings,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    music_dir: PathBuf,
    cards: HashMap<String, String>,
    #[serde(default)]
    volume: Option<RawVolumeConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct RawVolumeConfig {
    default: Option<f32>,
    #[serde(default)]
    quiet_hours: Vec<RawQuietWindow>,
}

#[derive(Debug, Deserialize)]
struct RawQuietWindow {
    start: String,
    end: String,
    volume: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct VolumeSettings {
    default_level: f32,
    quiet_windows: Vec<QuietWindow>,
}

#[derive(Debug, Clone)]
struct QuietWindow {
    start: NaiveTime,
    end: NaiveTime,
    level: f32,
}

impl MusicBoxConfig {
    pub fn from_reader<R: Read>(mut reader: R) -> Result<Self, ConfigError> {
        let mut buffer = String::new();
        reader.read_to_string(&mut buffer)?;
        let raw: RawConfig = toml::from_str(&buffer)?;
        Self::from_raw(raw)
    }

    pub fn music_dir(&self) -> &Path {
        &self.music_dir
    }

    pub fn volume_settings(&self) -> &VolumeSettings {
        &self.volume
    }

    fn from_raw(raw: RawConfig) -> Result<Self, ConfigError> {
        let RawConfig {
            music_dir,
            cards,
            volume,
        } = raw;
        let music_dir = music_dir;
        let mut parsed = HashMap::with_capacity(cards.len());
        for (card_hex, relative_path) in cards {
            let uid = CardUid::from_hex(card_hex.trim())?;
            let track_path = resolve_track_path(&music_dir, relative_path.trim());
            if parsed.insert(uid.clone(), track_path).is_some() {
                return Err(ConfigError::DuplicateCard(uid));
            }
        }
        let volume = VolumeSettings::from_raw(volume)?;
        Ok(Self {
            music_dir,
            cards: parsed,
            volume,
        })
    }

    pub fn into_library(self) -> Library {
        let (library, _) = self.into_parts();
        library
    }

    pub fn into_parts(self) -> (Library, VolumeSettings) {
        let library = build_library(self.cards);
        (library, self.volume)
    }

    pub fn library(&self) -> Library {
        build_library(self.cards.clone())
    }
}

fn build_library(entries: HashMap<CardUid, PathBuf>) -> Library {
    let tracks = entries
        .into_iter()
        .map(|(uid, path)| (uid, Track::new(path)))
        .collect();
    Library::new(tracks)
}

impl VolumeSettings {
    fn from_raw(raw: Option<RawVolumeConfig>) -> Result<Self, ConfigError> {
        let raw = raw.unwrap_or_default();
        let default = raw.default.unwrap_or(1.0);
        let default_level =
            validate_level(default).map_err(|reason| ConfigError::InvalidVolume {
                field: "volume.default",
                reason,
            })?;

        let mut quiet_windows = Vec::with_capacity(raw.quiet_hours.len());
        for (index, window) in raw.quiet_hours.into_iter().enumerate() {
            let start = parse_time(&window.start)
                .map_err(|reason| ConfigError::InvalidQuietHours { index, reason })?;
            let end = parse_time(&window.end)
                .map_err(|reason| ConfigError::InvalidQuietHours { index, reason })?;
            if start == end {
                return Err(ConfigError::InvalidQuietHours {
                    index,
                    reason: "start and end must differ".into(),
                });
            }

            let level_value = window
                .volume
                .ok_or_else(|| ConfigError::InvalidQuietHours {
                    index,
                    reason: "volume is required for each quiet hours entry".into(),
                })?;
            let level = validate_level(level_value)
                .map_err(|reason| ConfigError::InvalidQuietHours { index, reason })?;

            quiet_windows.push(QuietWindow { start, end, level });
        }

        Ok(Self {
            default_level,
            quiet_windows,
        })
    }

    pub fn default_level(&self) -> f32 {
        self.default_level
    }

    pub fn current_level(&self) -> f32 {
        let now = Local::now().time();
        self.level_for_time(now)
    }

    pub fn level_for_time(&self, time: NaiveTime) -> f32 {
        self.active_window(time)
            .map(|window| window.level)
            .unwrap_or(self.default_level)
    }

    fn active_window(&self, time: NaiveTime) -> Option<&QuietWindow> {
        self.quiet_windows
            .iter()
            .find(|window| window.contains(time))
    }
}

impl QuietWindow {
    fn contains(&self, time: NaiveTime) -> bool {
        if self.start <= self.end {
            time >= self.start && time < self.end
        } else {
            time >= self.start || time < self.end
        }
    }
}

fn validate_level(value: f32) -> Result<f32, String> {
    if !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "value {value} must be between 0.0 and 1.0 inclusive"
        ));
    }
    Ok(value)
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    let trimmed = value.trim();
    NaiveTime::parse_from_str(trimmed, "%H:%M")
        .map_err(|_| format!("{trimmed:?} must be in HH:MM 24h format"))
}

/// Adds a new card to the configuration file.
pub fn add_card_to_config(path: &Path, uid: &CardUid, track: &str) -> Result<(), ConfigEditError> {
    let mut doc = if path.exists() {
        let contents = fs::read_to_string(path).map_err(|source| ConfigEditError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        contents.parse::<DocumentMut>()?
    } else {
        let mut doc = DocumentMut::new();
        doc["music_dir"] = value("");
        doc["cards"] = table();
        doc
    };

    if !doc.as_table().contains_key("cards") {
        doc["cards"] = table();
    }

    let cards = doc["cards"]
        .as_table_mut()
        .ok_or(ConfigEditError::MissingCards)?;
    let uid_hex = uid.to_hex_lowercase();

    if cards.contains_key(&uid_hex) {
        return Err(ConfigEditError::Duplicate(uid.clone()));
    }

    cards.insert(&uid_hex, value(track));

    fs::write(path, doc.to_string()).map_err(|source| ConfigEditError::Write {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

/// Resolves the absolute path to a track.
fn resolve_track_path(music_dir: &Path, entry: &str) -> PathBuf {
    let path = PathBuf::from(entry);
    if path.is_absolute() || music_dir.as_os_str().is_empty() {
        path
    } else {
        normalize_join(music_dir, path)
    }
}

fn normalize_join(base: &Path, relative: PathBuf) -> PathBuf {
    base.join(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;
    use toml_edit::DocumentMut;

    #[test]
    fn builds_library_from_config() {
        let toml = r#"
music_dir = "/music"

[cards]
"0a0b" = "song1.mp3"
"0c0d" = "/absolute/song2.mp3"
"0e0f" = "nested/song3.ogg"
"#;

        let config = MusicBoxConfig::from_reader(toml.as_bytes()).unwrap();
        assert_eq!(config.music_dir(), Path::new("/music"));

        let library = config.into_library();

        assert_eq!(
            library
                .lookup(&CardUid::from_hex("0a0b").unwrap())
                .unwrap()
                .path(),
            Path::new("/music/song1.mp3")
        );
        assert_eq!(
            library
                .lookup(&CardUid::from_hex("0c0d").unwrap())
                .unwrap()
                .path(),
            Path::new("/absolute/song2.mp3")
        );
        assert_eq!(
            library
                .lookup(&CardUid::from_hex("0e0f").unwrap())
                .unwrap()
                .path(),
            Path::new("/music/nested/song3.ogg")
        );
    }

    #[test]
    fn invalid_card_uid_returns_error() {
        let toml = r#"
music_dir = "/music"

[cards]
"zz" = "song.mp3"
"#;

        let err = MusicBoxConfig::from_reader(toml.as_bytes()).unwrap_err();
        assert!(matches!(err, ConfigError::CardUid(_)));
    }

    #[test]
    fn add_card_to_config_creates_or_updates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("musicbox.toml");
        let uid = CardUid::from_hex("0a0b").unwrap();

        add_card_to_config(&path, &uid, "songs/track.mp3").unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let doc = contents.parse::<DocumentMut>().unwrap();
        assert_eq!(doc["music_dir"].as_str(), Some(""));
        assert_eq!(doc["cards"]["0a0b"].as_str(), Some("songs/track.mp3"));
    }

    #[test]
    fn add_card_to_config_rejects_duplicate_cards() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("musicbox.toml");
        std::fs::write(
            &path,
            r#"
music_dir = "/music"

[cards]
"0c0d" = "other.mp3"
"#
            .trim(),
        )
        .unwrap();

        let uid = CardUid::from_hex("0c0d").unwrap();
        let err = add_card_to_config(&path, &uid, "songs/new.mp3").unwrap_err();
        assert!(matches!(err, ConfigEditError::Duplicate(_)));
    }

    #[test]
    fn parses_volume_schedule() {
        let toml = r#"
music_dir = "/music"

[cards]
"0a0b" = "song1.mp3"

[volume]
default = 0.8

[[volume.quiet_hours]]
start = "20:00"
end = "07:30"
volume = 0.35
"#;

        let config = MusicBoxConfig::from_reader(toml.as_bytes()).unwrap();
        assert!((config.volume_settings().default_level() - 0.8).abs() < f32::EPSILON);

        let settings = config.volume_settings().clone();
        let evening = NaiveTime::from_hms_opt(21, 0, 0).unwrap();
        let morning = NaiveTime::from_hms_opt(6, 45, 0).unwrap();
        let midday = NaiveTime::from_hms_opt(12, 0, 0).unwrap();

        assert!((settings.level_for_time(evening) - 0.35).abs() < f32::EPSILON);
        assert!((settings.level_for_time(morning) - 0.35).abs() < f32::EPSILON);
        assert!((settings.level_for_time(midday) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_out_of_range_volume() {
        let toml = r#"
music_dir = "/music"

[cards]
"0a0b" = "song1.mp3"

[volume]
default = 1.2
"#;

        let err = MusicBoxConfig::from_reader(toml.as_bytes()).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidVolume { .. }));
    }

    #[test]
    fn rejects_invalid_quiet_hours() {
        let toml = r#"
music_dir = "/music"

[cards]
"0a0b" = "song1.mp3"

[[volume.quiet_hours]]
start = "10:00"
end = "10:00"
volume = 0.4
"#;

        let err = MusicBoxConfig::from_reader(toml.as_bytes()).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidQuietHours { .. }));
    }
}
