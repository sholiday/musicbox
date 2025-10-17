use clap::{Parser, ValueEnum, builder::ValueHint};
use musicbox::app::{RunLoopError, controller_from_config_path, run_until_shutdown};
use musicbox::audio::RodioPlayer;
use musicbox::controller::{AudioPlayer, PlayerError, Track};
use musicbox::reader::{NfcReader, ReaderError, ReaderEvent};
use std::path::PathBuf;
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
    #[error(transparent)]
    App(#[from] musicbox::app::AppError),
    #[error(transparent)]
    Loop(#[from] RunLoopError),
    #[error(transparent)]
    Reader(#[from] ReaderError),
}

#[derive(Debug, Parser)]
#[command(author, version, about = "NFC-triggered music player", long_about = None)]
struct Cli {
    #[arg(value_name = "CONFIG", value_hint = ValueHint::FilePath)]
    config: PathBuf,

    #[arg(long, default_value_t = 200, value_name = "MILLIS")]
    poll_interval_ms: u64,

    #[arg(long, value_enum, default_value_t = ReaderKind::Auto)]
    reader: ReaderKind,

    #[arg(long, help = "Disable audio playback (use silent mode)")]
    silent: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ReaderKind {
    Auto,
    Pcsc,
    Noop,
}

fn run() -> Result<(), RunError> {
    let cli = Cli::parse();

    let player = if cli.silent {
        PlayerBackend::Noop
    } else {
        match RodioPlayer::new() {
            Ok(player) => PlayerBackend::Rodio(player),
            Err(err) => {
                eprintln!("Audio backend unavailable ({err}). Falling back to silent playback.");
                PlayerBackend::Noop
            }
        }
    };

    let mut controller = controller_from_config_path(&cli.config, player)?;
    let mut reader = select_reader(cli.reader, Duration::from_millis(cli.poll_interval_ms))?;

    println!("Loaded configuration from {}", cli.config.display());
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

fn select_reader(kind: ReaderKind, poll: Duration) -> Result<Box<dyn NfcReader>, ReaderError> {
    match kind {
        ReaderKind::Noop => Ok(Box::new(NoopReader::default())),
        ReaderKind::Pcsc => build_pcsc_reader(poll),
        ReaderKind::Auto => match build_pcsc_reader(poll) {
            Ok(reader) => Ok(reader),
            Err(err) => {
                eprintln!("PC/SC reader unavailable ({err}); falling back to noop reader.");
                Ok(Box::new(NoopReader::default()))
            }
        },
    }
}

#[cfg(feature = "nfc-pcsc")]
fn build_pcsc_reader(poll: Duration) -> Result<Box<dyn NfcReader>, ReaderError> {
    let reader = musicbox::reader::pcsc_backend::PcscReader::new(poll)?;
    Ok(Box::new(reader))
}

#[cfg(not(feature = "nfc-pcsc"))]
fn build_pcsc_reader(_poll: Duration) -> Result<Box<dyn NfcReader>, ReaderError> {
    Err(ReaderError::backend(
        "pcsc support not built; recompile with `--features nfc-pcsc`",
    ))
}
