use musicbox::app::{
    ProcessOutcome, RunLoopError, controller_from_config_path, process_next_event,
};
use musicbox::controller::{AudioPlayer, PlayerError, Track};
use musicbox::reader::{NfcReader, ReaderError, ReaderEvent};
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
}

fn run() -> Result<(), RunError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "musicbox".into());
    let config_path = match args.next() {
        Some(path) => path,
        None => return Err(RunError::MissingConfigPath { program }),
    };

    let mut controller = controller_from_config_path(&config_path, NoopPlayer)?;
    let mut reader = NoopReader::default();

    println!("Loaded configuration from {}", config_path);
    println!("Awaiting NFC interactions (reader not connected in this environment).");

    match process_next_event(&mut controller, &mut reader)? {
        ProcessOutcome::Action(action) => println!("Simulated action: {:?}", action),
        ProcessOutcome::NoEvent => println!("No reader events to process."),
        ProcessOutcome::Shutdown => println!("Reader requested shutdown."),
    }

    Ok(())
}

struct NoopPlayer;

impl AudioPlayer for NoopPlayer {
    fn play(&mut self, track: &Track) -> Result<(), PlayerError> {
        println!("Would play track: {}", track.path().display());
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlayerError> {
        println!("Would stop playback");
        Ok(())
    }
}

#[derive(Default)]
struct NoopReader;

impl NfcReader for NoopReader {
    fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
        Ok(ReaderEvent::Shutdown)
    }
}
