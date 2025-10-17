use musicbox::app::{RunLoopError, controller_from_config_path, run_until_shutdown};
use musicbox::audio::RodioPlayer;
use musicbox::controller::{AudioPlayer, PlayerError, Track};
use musicbox::reader::{NfcReader, ReaderError, ReaderEvent};
use std::time::Duration;
use thiserror::Error;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[derive(Debug, Error)]
enum RunError {
    #[error("usage: {program} <config-path>")]
    MissingConfigPath { program: String },
    #[error(transparent)]
    App(#[from] musicbox::app::AppError),
    #[error(transparent)]
    Loop(#[from] RunLoopError),
    #[error(transparent)]
    Reader(#[from] ReaderError),
}

fn run() -> Result<(), RunError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "musicbox".into());
    let config_path = match args.next() {
        Some(path) => path,
        None => return Err(RunError::MissingConfigPath { program }),
    };

    let player = match RodioPlayer::new() {
        Ok(player) => PlayerBackend::Rodio(player),
        Err(err) => {
            eprintln!("Audio backend unavailable ({err}). Falling back to silent playback.");
            PlayerBackend::Noop
        }
    };

    let mut controller = controller_from_config_path(&config_path, player)?;
    let mut reader = select_reader()?;

    println!("Loaded configuration from {}", config_path);
    println!("Awaiting NFC interactions (reader not connected in this environment).");

    run_until_shutdown(
        &mut controller,
        &mut reader,
        |action| println!("Simulated action: {:?}", action),
        || std::thread::sleep(Duration::from_millis(200)),
    )?;

    println!("Reader requested shutdown. Exiting.");

    Ok(())
}

enum PlayerBackend {
    Rodio(RodioPlayer),
    Noop,
}

impl AudioPlayer for PlayerBackend {
    fn play(&mut self, track: &Track) -> Result<(), PlayerError> {
        match self {
            PlayerBackend::Rodio(player) => player.play(track),
            PlayerBackend::Noop => {
                println!("[silent] Would play track: {}", track.path().display());
                Ok(())
            }
        }
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        match self {
            PlayerBackend::Rodio(player) => player.stop(),
            PlayerBackend::Noop => {
                println!("[silent] Would stop playback");
                Ok(())
            }
        }
    }
}

#[derive(Default)]
struct NoopReader;

impl NfcReader for NoopReader {
    fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
        Ok(ReaderEvent::Shutdown)
    }
}

fn select_reader() -> Result<Box<dyn NfcReader>, ReaderError> {
    #[cfg(feature = "nfc-pcsc")]
    {
        let reader = musicbox::reader::pcsc_backend::PcscReader::new(Duration::from_millis(200))?;
        return Ok(Box::new(reader));
    }
    #[cfg(not(feature = "nfc-pcsc"))]
    {
        Ok(Box::new(NoopReader::default()))
    }
}
