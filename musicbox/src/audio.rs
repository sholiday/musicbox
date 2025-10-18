use crate::controller::{AudioPlayer, PlayerError, Track};

#[cfg(feature = "audio-rodio")]
mod rodio_backend {
    use super::*;
    use rodio::source::Source;
    use rodio::{OutputStream, OutputStreamHandle, Sink};
    use std::io::BufReader;
    use std::path::Path;

    pub struct RodioPlayer {
        _stream: OutputStream,
        handle: OutputStreamHandle,
        sink: Sink,
    }

    impl RodioPlayer {
        pub fn new() -> Result<Self, PlayerError> {
            let (stream, handle) =
                OutputStream::try_default().map_err(|err| PlayerError::Backend {
                    message: format!("failed to open output stream: {err}"),
                })?;
            let sink = Sink::try_new(&handle).map_err(|err| PlayerError::Backend {
                message: format!("failed to create sink: {err}"),
            })?;
            Ok(Self {
                _stream: stream,
                handle,
                sink,
            })
        }

        fn load_track(path: &Path) -> Result<impl Source<Item = f32> + Send, PlayerError> {
            let file = std::fs::File::open(path).map_err(|err| PlayerError::Backend {
                message: format!("failed to open track {path:?}: {err}"),
            })?;
            let decoder =
                rodio::Decoder::new(BufReader::new(file)).map_err(|err| PlayerError::Backend {
                    message: format!("failed to decode track {path:?}: {err}"),
                })?;
            Ok(decoder.convert_samples())
        }

        fn reset_sink(&mut self) -> Result<(), PlayerError> {
            self.sink = Sink::try_new(&self.handle).map_err(|err| PlayerError::Backend {
                message: format!("failed to reset sink: {err}"),
            })?;
            Ok(())
        }
    }

    impl AudioPlayer for RodioPlayer {
        fn play(&mut self, track: &Track) -> Result<(), PlayerError> {
            self.reset_sink()?;
            let source = Self::load_track(track.path())?;
            self.sink.append(source);
            self.sink.play();
            Ok(())
        }

        fn stop(&mut self) -> Result<(), PlayerError> {
            if !self.sink.empty() {
                self.sink.stop();
                self.reset_sink()?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::Path;

        #[test]
        fn load_track_returns_error_for_missing_file() {
            let err = RodioPlayer::load_track(Path::new("./does-not-exist.ogg")).unwrap_err();
            assert!(matches!(err, PlayerError::Backend { .. }));
        }
    }
}

#[cfg(not(feature = "audio-rodio"))]
mod rodio_backend {
    use super::*;

    #[derive(Debug, Default)]
    pub struct RodioPlayer;

    impl RodioPlayer {
        pub fn new() -> Result<Self, PlayerError> {
            Err(PlayerError::Backend {
                message: "rodio backend disabled; enable the `audio-rodio` feature to use it"
                    .into(),
            })
        }
    }

    impl AudioPlayer for RodioPlayer {
        fn play(&mut self, _track: &Track) -> Result<(), PlayerError> {
            Err(PlayerError::Backend {
                message: "rodio backend disabled".into(),
            })
        }

        fn stop(&mut self) -> Result<(), PlayerError> {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn new_reports_disabled_backend() {
            match RodioPlayer::new() {
                Ok(_) => panic!("expected error"),
                Err(err) => assert!(matches!(err, PlayerError::Backend { .. })),
            }
        }

        #[test]
        fn stop_is_noop() {
            let mut player = RodioPlayer::default();
            player.stop().expect("stop should succeed");
        }
    }
}

pub use rodio_backend::RodioPlayer;
