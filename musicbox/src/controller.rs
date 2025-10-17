use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CardUid(pub Vec<u8>);

impl CardUid {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub path: PathBuf,
}

impl Track {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Library {
    tracks: HashMap<CardUid, Track>,
}

impl Library {
    pub fn new(entries: HashMap<CardUid, Track>) -> Self {
        Self { tracks: entries }
    }

    pub fn lookup(&self, uid: &CardUid) -> Option<&Track> {
        self.tracks.get(uid)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ControllerError {
    #[error("track not found for card")]
    TrackNotFound,
    #[error("audio player error: {0}")]
    Audio(#[from] PlayerError),
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("audio backend failure: {message}")]
    Backend { message: String },
}

pub trait AudioPlayer {
    fn play(&mut self, track: &Track) -> Result<(), PlayerError>;
    fn stop(&mut self) -> Result<(), PlayerError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerAction {
    Started {
        card: CardUid,
        track: Track,
    },
    Stopped {
        card: CardUid,
        track: Track,
    },
    Switched {
        from_card: CardUid,
        from_track: Track,
        to_card: CardUid,
        to_track: Track,
    },
}

struct ActiveTrack {
    card: CardUid,
    track: Track,
}

pub struct MusicBoxController<P: AudioPlayer> {
    library: Library,
    player: P,
    active: Option<ActiveTrack>,
}

impl<P: AudioPlayer> MusicBoxController<P> {
    pub fn new(library: Library, player: P) -> Self {
        Self {
            library,
            player,
            active: None,
        }
    }

    pub fn handle_card(&mut self, uid: &CardUid) -> Result<ControllerAction, ControllerError> {
        if let Some(active) = &self.active {
            if &active.card == uid {
                self.player.stop()?;
                let stopped = ControllerAction::Stopped {
                    card: active.card.clone(),
                    track: active.track.clone(),
                };
                self.active = None;
                return Ok(stopped);
            }
        }

        let track = self
            .library
            .lookup(uid)
            .cloned()
            .ok_or(ControllerError::TrackNotFound)?;

        let action = if let Some(active) = self.active.take() {
            self.player.stop()?;
            self.player.play(&track)?;
            let action = ControllerAction::Switched {
                from_card: active.card.clone(),
                from_track: active.track.clone(),
                to_card: uid.clone(),
                to_track: track.clone(),
            };
            self.active = Some(ActiveTrack {
                card: uid.clone(),
                track: track.clone(),
            });
            action
        } else {
            self.player.play(&track)?;
            self.active = Some(ActiveTrack {
                card: uid.clone(),
                track: track.clone(),
            });
            ControllerAction::Started {
                card: uid.clone(),
                track: track.clone(),
            }
        };

        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

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
        fn play(&mut self, track: &Track) -> Result<(), PlayerError> {
            self.calls.borrow_mut().push(Call::Play(track.path.clone()));
            Ok(())
        }

        fn stop(&mut self) -> Result<(), PlayerError> {
            self.calls.borrow_mut().push(Call::Stop);
            Ok(())
        }
    }

    fn library_with(entries: Vec<(CardUid, &str)>) -> Library {
        let map = entries
            .into_iter()
            .map(|(uid, path)| (uid, Track::new(PathBuf::from(path))))
            .collect();
        Library::new(map)
    }

    fn uid(bytes: &[u8]) -> CardUid {
        CardUid::new(bytes.to_vec())
    }

    #[test]
    fn tapping_card_starts_associated_track() {
        let player = MockPlayer::new();
        let library = library_with(vec![(uid(&[1, 2]), "song1.mp3")]);
        let mut controller = MusicBoxController::new(library, player.clone());

        let action = controller.handle_card(&uid(&[1, 2])).unwrap();

        assert_eq!(
            action,
            ControllerAction::Started {
                card: uid(&[1, 2]),
                track: Track::new(PathBuf::from("song1.mp3"))
            }
        );
        assert_eq!(player.calls(), vec![Call::Play(PathBuf::from("song1.mp3"))]);
    }

    #[test]
    fn tapping_same_card_stops_playback() {
        let player = MockPlayer::new();
        let library = library_with(vec![(uid(&[1, 2]), "song1.mp3")]);
        let mut controller = MusicBoxController::new(library, player.clone());

        controller.handle_card(&uid(&[1, 2])).unwrap();
        let action = controller.handle_card(&uid(&[1, 2])).unwrap();

        assert_eq!(
            action,
            ControllerAction::Stopped {
                card: uid(&[1, 2]),
                track: Track::new(PathBuf::from("song1.mp3"))
            }
        );
        assert_eq!(
            player.calls(),
            vec![Call::Play(PathBuf::from("song1.mp3")), Call::Stop,]
        );
    }

    #[test]
    fn tapping_different_card_switches_tracks() {
        let player = MockPlayer::new();
        let library = library_with(vec![
            (uid(&[1, 2]), "song1.mp3"),
            (uid(&[3, 4]), "song2.mp3"),
        ]);
        let mut controller = MusicBoxController::new(library, player.clone());

        controller.handle_card(&uid(&[1, 2])).unwrap();
        let action = controller.handle_card(&uid(&[3, 4])).unwrap();

        assert_eq!(
            action,
            ControllerAction::Switched {
                from_card: uid(&[1, 2]),
                from_track: Track::new(PathBuf::from("song1.mp3")),
                to_card: uid(&[3, 4]),
                to_track: Track::new(PathBuf::from("song2.mp3")),
            }
        );
        assert_eq!(
            player.calls(),
            vec![
                Call::Play(PathBuf::from("song1.mp3")),
                Call::Stop,
                Call::Play(PathBuf::from("song2.mp3")),
            ]
        );
    }

    #[test]
    fn unknown_card_causes_error_without_audio_calls() {
        let player = MockPlayer::new();
        let library = library_with(vec![(uid(&[1, 2]), "song1.mp3")]);
        let mut controller = MusicBoxController::new(library, player.clone());

        let err = controller.handle_card(&uid(&[9, 9])).unwrap_err();

        assert!(matches!(err, ControllerError::TrackNotFound));
        assert!(player.calls().is_empty());
    }
}
