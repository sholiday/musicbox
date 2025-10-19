use crate::controller::{AudioPlayer, PlayerError, Track};

#[cfg(feature = "audio-rodio")]
mod rodio_backend {
    use super::*;
    use rodio::{OutputStream, OutputStreamBuilder, Sink};
    use std::fs::File;
    use std::path::Path;

    pub struct RodioPlayer {
        stream: OutputStream,
        sink: Sink,
    }

    impl RodioPlayer {
        pub fn new() -> Result<Self, PlayerError> {
            let stream =
                OutputStreamBuilder::open_default_stream().map_err(|err| PlayerError::Backend {
                    message: format!("failed to open output stream: {err}"),
                })?;
            let sink = Sink::connect_new(stream.mixer());
            Ok(Self { stream, sink })
        }

        fn load_track(
            path: &Path,
        ) -> Result<rodio::Decoder<std::io::BufReader<File>>, PlayerError> {
            let file = File::open(path).map_err(|err| PlayerError::Backend {
                message: format!("failed to open track {path:?}: {err}"),
            })?;
            let decoder = rodio::Decoder::try_from(file).map_err(|err| PlayerError::Backend {
                message: format!("failed to decode track {path:?}: {err}"),
            })?;
            Ok(decoder)
        }

        fn reset_sink(&mut self) {
            self.sink = Sink::connect_new(self.stream.mixer());
        }
    }

    impl AudioPlayer for RodioPlayer {
        fn play(&mut self, track: &Track) -> Result<(), PlayerError> {
            self.reset_sink();
            let source = Self::load_track(track.path())?;
            self.sink.append(source);
            self.sink.play();
            Ok(())
        }

        fn stop(&mut self) -> Result<(), PlayerError> {
            if !self.sink.empty() {
                self.sink.stop();
                self.reset_sink();
            }
            Ok(())
        }

        fn wait_until_done(&mut self) -> Result<(), PlayerError> {
            self.sink.sleep_until_end();
            self.reset_sink();
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::Path;

        #[test]
        fn load_track_returns_error_for_missing_file() {
            let result = RodioPlayer::load_track(Path::new("./does-not-exist.ogg"));
            assert!(matches!(result, Err(PlayerError::Backend { .. })));
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
