use crate::config::{ConfigError, MusicBoxConfig};
use crate::controller::{AudioPlayer, ControllerAction, ControllerError, MusicBoxController};
use crate::reader::{NfcReader, ReaderError, ReaderEvent};
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

#[derive(Debug, thiserror::Error)]
pub enum RunLoopError {
    #[error("reader error: {0}")]
    Reader(#[from] ReaderError),
    #[error("controller error: {0}")]
    Controller(#[from] ControllerError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    Action(ControllerAction),
    NoEvent,
    Shutdown,
}

pub fn process_next_event<R, P>(
    controller: &mut MusicBoxController<P>,
    reader: &mut R,
) -> Result<ProcessOutcome, RunLoopError>
where
    R: NfcReader,
    P: AudioPlayer,
{
    let event = reader.next_event()?;
    match event {
        ReaderEvent::CardPresent { uid } => {
            let action = controller.handle_card(&uid)?;
            Ok(ProcessOutcome::Action(action))
        }
        ReaderEvent::Idle => Ok(ProcessOutcome::NoEvent),
        ReaderEvent::Shutdown => Ok(ProcessOutcome::Shutdown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{
        CardUid, ControllerAction, ControllerError, Library, MusicBoxController, Track,
    };
    use crate::reader::{NfcReader, ReaderError, ReaderEvent};
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
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

    #[derive(Clone)]
    struct ScriptedReader {
        events: Rc<RefCell<VecDeque<Result<ReaderEvent, ReaderError>>>>,
    }

    impl ScriptedReader {
        fn from_events(events: Vec<ReaderEvent>) -> Self {
            let sequence = events.into_iter().map(Ok).collect();
            Self::new(sequence)
        }

        fn new(sequence: Vec<Result<ReaderEvent, ReaderError>>) -> Self {
            Self {
                events: Rc::new(RefCell::new(sequence.into())),
            }
        }
    }

    impl NfcReader for ScriptedReader {
        fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
            self.events
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(ReaderEvent::Shutdown))
        }
    }

    fn write_config(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("create temp config");
        std::io::Write::write_all(&mut file, contents.as_bytes()).expect("write config");
        file
    }

    fn controller_with_tracks(
        entries: Vec<(&str, &str)>,
        player: MockPlayer,
    ) -> MusicBoxController<MockPlayer> {
        let tracks: HashMap<_, _> = entries
            .into_iter()
            .map(|(uid_hex, path)| {
                (
                    CardUid::from_hex(uid_hex).unwrap(),
                    Track::new(PathBuf::from(path)),
                )
            })
            .collect();
        MusicBoxController::new(Library::new(tracks), player)
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

    #[test]
    fn process_next_event_triggers_controller_on_card_present() {
        let player = MockPlayer::new();
        let mut controller =
            controller_with_tracks(vec![("0102", "/music/song1.mp3")], player.clone());
        let mut reader = ScriptedReader::from_events(vec![ReaderEvent::CardPresent {
            uid: CardUid::from_hex("0102").unwrap(),
        }]);

        let outcome = process_next_event(&mut controller, &mut reader).unwrap();

        assert_eq!(
            outcome,
            ProcessOutcome::Action(ControllerAction::Started {
                card: CardUid::from_hex("0102").unwrap(),
                track: Track::new(PathBuf::from("/music/song1.mp3")),
            })
        );
        assert_eq!(
            player.calls(),
            vec![Call::Play(PathBuf::from("/music/song1.mp3"))]
        );
    }

    #[test]
    fn process_next_event_returns_no_event_on_idle() {
        let player = MockPlayer::new();
        let mut controller =
            controller_with_tracks(vec![("0102", "/music/song1.mp3")], player.clone());
        let mut reader = ScriptedReader::from_events(vec![ReaderEvent::Idle]);

        let outcome = process_next_event(&mut controller, &mut reader).unwrap();

        assert_eq!(outcome, ProcessOutcome::NoEvent);
        assert!(player.calls().is_empty());
    }

    #[test]
    fn process_next_event_returns_shutdown() {
        let player = MockPlayer::new();
        let mut controller =
            controller_with_tracks(vec![("0102", "/music/song1.mp3")], player.clone());
        let mut reader = ScriptedReader::from_events(vec![ReaderEvent::Shutdown]);

        let outcome = process_next_event(&mut controller, &mut reader).unwrap();

        assert_eq!(outcome, ProcessOutcome::Shutdown);
        assert!(player.calls().is_empty());
    }

    #[test]
    fn process_next_event_returns_controller_error() {
        let mut controller = controller_with_tracks(vec![], MockPlayer::new());
        let mut reader = ScriptedReader::from_events(vec![ReaderEvent::CardPresent {
            uid: CardUid::from_hex("0304").unwrap(),
        }]);

        let err = process_next_event(&mut controller, &mut reader).unwrap_err();

        assert!(matches!(
            err,
            RunLoopError::Controller(ControllerError::TrackNotFound)
        ));
    }

    #[test]
    fn process_next_event_returns_reader_error() {
        let player = MockPlayer::new();
        let mut controller = controller_with_tracks(vec![("0102", "/music/song1.mp3")], player);
        let mut reader = ScriptedReader::new(vec![Err(ReaderError::Backend {
            message: "boom".into(),
        })]);

        let err = process_next_event(&mut controller, &mut reader).unwrap_err();

        assert!(matches!(
            err,
            RunLoopError::Reader(ReaderError::Backend { .. })
        ));
    }
}
