use musicbox::app::controller_from_config_path;
use musicbox::controller::{AudioPlayer, PlayerError, Track};
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
}

fn run() -> Result<(), RunError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "musicbox".into());
    let config_path = match args.next() {
        Some(path) => path,
        None => return Err(RunError::MissingConfigPath { program }),
    };

    let _controller = controller_from_config_path(&config_path, NoopPlayer)?;
    println!("Loaded configuration from {}", config_path);
    println!("Awaiting NFC interactions (reader not connected in this environment).");

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
