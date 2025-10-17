use crate::config::{ConfigError, MusicBoxConfig};
use crate::controller::{AudioPlayer, MusicBoxController};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("failed to open config file {path:?}: {source}")]
    OpenConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
}

pub fn controller_from_config_path<P: AudioPlayer>(
    path: impl AsRef<Path>,
    player: P,
) -> Result<MusicBoxController<P>, AppError> {
    let path_ref = path.as_ref();
    let file = File::open(path_ref).map_err(|source| AppError::OpenConfig {
        path: path_ref.into(),
        source,
    })?;
    let config = MusicBoxConfig::from_reader(file)?;
    let library = config.into_library();
    Ok(MusicBoxController::new(library, player))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{CardUid, ControllerAction, Track};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use tempfile::NamedTempFile;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Play(PathBuf),
        Stop,
    }

    #[derive(Clone)]
    struct MockPlayer {
        calls: Rc<RefCell<Vec<Call>>>,
    }

    impl MockPlayer {
        fn new() -> Self {
            Self {
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.borrow().clone()
        }
    }

    impl AudioPlayer for MockPlayer {
        fn play(&mut self, track: &Track) -> Result<(), crate::controller::PlayerError> {
            self.calls
                .borrow_mut()
                .push(Call::Play(track.path().to_path_buf()));
            Ok(())
        }

        fn stop(&mut self) -> Result<(), crate::controller::PlayerError> {
            self.calls.borrow_mut().push(Call::Stop);
            Ok(())
        }
    }

    fn write_config(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("create temp config");
        std::io::Write::write_all(&mut file, contents.as_bytes()).expect("write config");
        file
    }

    #[test]
    fn builds_controller_for_configured_cards() {
        let config_toml = r#"
music_dir = "/music"

[cards]
"0102" = "song1.mp3"
"0304" = "nested/song2.mp3"
"#;
        let file = write_config(config_toml);
        let player = MockPlayer::new();

        let mut controller =
            controller_from_config_path(file.path(), player.clone()).expect("load config");

        let action = controller
            .handle_card(&CardUid::from_hex("0102").unwrap())
            .expect("play");

        assert_eq!(
            action,
            ControllerAction::Started {
                card: CardUid::from_hex("0102").unwrap(),
                track: Track::new(PathBuf::from("/music/song1.mp3")),
            }
        );
        assert_eq!(
            player.calls(),
            vec![Call::Play(PathBuf::from("/music/song1.mp3"))]
        );
    }

    #[test]
    fn errors_when_file_missing() {
        match controller_from_config_path("/does/not/exist/config.toml", MockPlayer::new()) {
            Ok(_) => panic!("expected error"),
            Err(err) => assert!(matches!(err, AppError::OpenConfig { .. })),
        }
    }
}
