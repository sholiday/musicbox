use clap::{Args, Parser, Subcommand, ValueEnum, builder::ValueHint};
use musicbox::app::{RunLoopError, controller_from_config_path, run_until_shutdown};
use musicbox::audio::RodioPlayer;
use musicbox::config::{self, ConfigEditError};
use musicbox::controller::{AudioPlayer, CardUid, CardUidParseError, PlayerError, Track};
use musicbox::reader::{NfcReader, ReaderError, ReaderEvent};
use musicbox::telemetry::{self, SharedStatus};
#[cfg(feature = "debug-http")]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

fn main() {
    telemetry::init_logging();

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
    #[error(transparent)]
    Tag(#[from] TagError),
    #[error("audio player error: {0}")]
    Player(#[from] PlayerError),
    #[error("configuration path required")]
    MissingConfig,
}

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "NFC-triggered music player",
    long_about = None,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[arg(value_name = "CONFIG", value_hint = ValueHint::FilePath)]
    config: Option<PathBuf>,

    #[arg(long, default_value_t = 200, value_name = "MILLIS")]
    poll_interval_ms: u64,

    #[arg(long, value_enum, default_value_t = ReaderKind::Auto)]
    reader: ReaderKind,

    #[arg(long, help = "Disable audio playback (use silent mode)")]
    silent: bool,

    #[cfg(feature = "debug-http")]
    #[arg(long, value_name = "ADDR", value_hint = ValueHint::Hostname)]
    debug_http: Option<SocketAddr>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(subcommand)]
    Tag(TagCommand),
    #[command(subcommand)]
    Manual(ManualCommand),
}

#[derive(Debug, Subcommand)]
enum TagCommand {
    Add(TagAddArgs),
}

#[derive(Debug, Subcommand)]
enum ManualCommand {
    Trigger(ManualTriggerArgs),
}

#[derive(Debug, Args)]
struct ManualTriggerArgs {
    #[arg(long, value_name = "CONFIG", value_hint = ValueHint::FilePath)]
    config: PathBuf,
    #[arg(value_name = "UID", help = "Hex-encoded card UID (no spaces)")]
    card: String,
}

#[derive(Debug, Args)]
struct TagAddArgs {
    #[arg(long, value_name = "CONFIG", value_hint = ValueHint::FilePath)]
    config: PathBuf,

    #[arg(long, value_name = "TRACK", value_hint = ValueHint::FilePath)]
    track: PathBuf,

    #[arg(long, value_name = "UID", help = "Hex-encoded card UID (no spaces)")]
    card: Option<String>,

    #[arg(
        long,
        value_enum,
        value_name = "KIND",
        help = "Reader backend override"
    )]
    reader: Option<ReaderKind>,

    #[arg(
        long,
        value_name = "MILLIS",
        help = "Override poll interval in milliseconds while waiting for a card"
    )]
    poll_interval_ms: Option<u64>,

    #[arg(long, help = "Skip writing metadata to the NFC tag")]
    skip_tag_write: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ReaderKind {
    Auto,
    Pcsc,
    Noop,
}

#[derive(Debug, Error)]
enum TagError {
    #[error("card UID must be provided when using the noop reader")]
    CardUidRequired,
    #[error("invalid card uid: {0}")]
    CardUidParse(#[from] CardUidParseError),
    #[error("reader error: {0}")]
    Reader(#[from] ReaderError),
    #[error("reader shut down before a card was detected")]
    ReaderShutdown,
    #[error(transparent)]
    Config(#[from] ConfigEditError),
    #[error("track path {0:?} is not valid UTF-8")]
    InvalidTrackPath(PathBuf),
}

fn run() -> Result<(), RunError> {
    let cli = Cli::parse();

    #[cfg(feature = "debug-http")]
    let Cli {
        config,
        poll_interval_ms,
        reader,
        silent,
        debug_http,
        command,
    } = cli;

    #[cfg(not(feature = "debug-http"))]
    let Cli {
        config,
        poll_interval_ms,
        reader,
        silent,
        command,
    } = cli;

    match command {
        Some(Command::Tag(tag_command)) => {
            handle_tag_command(tag_command, reader, poll_interval_ms)?;
        }
        Some(Command::Manual(manual_command)) => {
            handle_manual_command(manual_command, silent)?;
        }
        None => {
            let config_path = config.ok_or(RunError::MissingConfig)?;
            run_player_main(
                config_path,
                poll_interval_ms,
                reader,
                silent,
                #[cfg(feature = "debug-http")]
                debug_http,
            )?;
        }
    }

    Ok(())
}

fn run_player_main(
    config_path: PathBuf,
    poll_interval_ms: u64,
    reader_kind: ReaderKind,
    silent: bool,
    #[cfg(feature = "debug-http")] debug_http: Option<SocketAddr>,
) -> Result<(), RunError> {
    let player = if silent {
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

    let mut controller = controller_from_config_path(&config_path, player)?;
    let mut reader = select_reader(reader_kind, Duration::from_millis(poll_interval_ms))?;

    let status = SharedStatus::default();
    let idle_status = status.clone();

    #[cfg(feature = "debug-http")]
    if let Some(addr) = debug_http {
        let server_status = status.clone();
        std::thread::spawn(move || {
            if let Err(err) = musicbox::web::serve(server_status, addr) {
                tracing::error!(?err, "debug server terminated");
            }
        });
    }

    println!("Loaded configuration from {}", config_path.display());
    println!("Awaiting NFC interactions (reader not connected in this environment).");

    let sleep_duration = Duration::from_millis(poll_interval_ms);

    run_until_shutdown(
        &mut controller,
        &mut reader,
        |action| {
            println!("Controller action: {:?}", action);
            status.record_action(action.clone());
            tracing::info!(?action, "controller action");
        },
        || {
            idle_status.record_idle();
            std::thread::sleep(sleep_duration);
        },
    )?;

    println!("Reader requested shutdown. Exiting.");
    tracing::info!(snapshot = ?status.snapshot(), "final status");

    Ok(())
}

fn handle_tag_command(
    command: TagCommand,
    default_reader: ReaderKind,
    default_poll_ms: u64,
) -> Result<(), TagError> {
    match command {
        TagCommand::Add(args) => handle_tag_add(args, default_reader, default_poll_ms),
    }
}

fn handle_tag_add(
    args: TagAddArgs,
    default_reader: ReaderKind,
    default_poll_ms: u64,
) -> Result<(), TagError> {
    let reader_kind = args.reader.unwrap_or(default_reader);
    let poll_ms = args.poll_interval_ms.unwrap_or(default_poll_ms);

    if args.card.is_none() && matches!(reader_kind, ReaderKind::Noop) {
        return Err(TagError::CardUidRequired);
    }

    let track = path_to_string(&args.track)?;

    let uid = if let Some(card_hex) = args.card {
        CardUid::from_hex(card_hex.trim())?
    } else {
        acquire_card_uid(reader_kind, Duration::from_millis(poll_ms))?
    };

    config::add_card_to_config(&args.config, &uid, &track)?;

    println!(
        "Mapped card {} to {} in {}",
        uid,
        track,
        args.config.display()
    );

    if args.skip_tag_write {
        println!("Skipping NFC tag write (per --skip-tag-write).");
    } else if let Err(err) =
        attempt_tag_write(reader_kind, Duration::from_millis(poll_ms), &uid, &track)
    {
        tracing::warn!(?err, "failed to write NFC tag; config still updated");
    }

    Ok(())
}

fn path_to_string(path: &Path) -> Result<String, TagError> {
    path.to_str()
        .map(|s| s.to_owned())
        .ok_or_else(|| TagError::InvalidTrackPath(path.to_path_buf()))
}

fn acquire_card_uid(reader_kind: ReaderKind, poll: Duration) -> Result<CardUid, TagError> {
    let mut reader = select_reader(reader_kind, poll)?;
    loop {
        match reader.next_event()? {
            ReaderEvent::CardPresent { uid } => return Ok(uid),
            ReaderEvent::Idle => continue,
            ReaderEvent::Shutdown => return Err(TagError::ReaderShutdown),
        }
    }
}

fn attempt_tag_write(
    reader_kind: ReaderKind,
    poll: Duration,
    uid: &CardUid,
    track: &str,
) -> Result<(), TagError> {
    let _ = (reader_kind, poll, uid, track);
    #[cfg(feature = "nfc-pcsc")]
    {
        println!(
            "Writing track metadata to NFC tag {} via PC/SC is not implemented yet.",
            uid
        );
    }
    #[cfg(not(feature = "nfc-pcsc"))]
    {
        println!(
            "Tag writing unavailable without the `nfc-pcsc` feature; config has still been updated."
        );
    }
    Ok(())
}

fn handle_manual_command(command: ManualCommand, silent: bool) -> Result<(), RunError> {
    match command {
        ManualCommand::Trigger(args) => handle_manual_trigger(args, silent),
    }
}

fn handle_manual_trigger(args: ManualTriggerArgs, silent: bool) -> Result<(), RunError> {
    let player = if silent {
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

    let mut controller = controller_from_config_path(&args.config, player)?;

    let uid = CardUid::from_hex(args.card.trim())
        .map_err(TagError::CardUidParse)
        .map_err(RunError::Tag)?;

    let action = controller
        .handle_card(&uid)
        .map_err(RunLoopError::from)
        .map_err(RunError::Loop)?;

    println!("Manual trigger produced action: {:?}", action);
    controller.wait_for_player()?;

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

    fn wait_until_done(&mut self) -> Result<(), PlayerError> {
        match self {
            PlayerBackend::Rodio(player) => player.wait_until_done(),
            PlayerBackend::Noop => Ok(()),
        }
    }
}

struct NoopReader {
    shutdown_next: bool,
    shutdown_sent: bool,
}

impl Default for NoopReader {
    fn default() -> Self {
        let shutdown_next = std::env::var("MUSICBOX_NOOP_SHUTDOWN")
            .map(|val| val != "0")
            .unwrap_or(false);
        Self {
            shutdown_next,
            shutdown_sent: false,
        }
    }
}

impl NfcReader for NoopReader {
    fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
        if self.shutdown_next && !self.shutdown_sent {
            self.shutdown_sent = true;
            return Ok(ReaderEvent::Shutdown);
        }
        Ok(ReaderEvent::Idle)
    }
}

fn select_reader(kind: ReaderKind, poll: Duration) -> Result<Box<dyn NfcReader>, ReaderError> {
    match kind {
        ReaderKind::Noop => Ok(Box::new(NoopReader::default())),
        ReaderKind::Pcsc => build_pcsc_reader(poll),
        ReaderKind::Auto => match build_pcsc_reader(poll) {
            Ok(reader) => Ok(reader),
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "PC/SC reader unavailable; falling back to noop reader"
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_reader_noop() {
        // std::env mutations are unsafe on recent toolchains, so gate them explicitly in tests.
        unsafe {
            std::env::set_var("MUSICBOX_NOOP_SHUTDOWN", "1");
        }
        let reader = select_reader(ReaderKind::Noop, Duration::from_millis(1)).unwrap();
        let mut reader = reader;
        let event = reader.next_event().unwrap();
        assert!(matches!(event, ReaderEvent::Shutdown));
        unsafe {
            std::env::remove_var("MUSICBOX_NOOP_SHUTDOWN");
        }
    }

    #[cfg(not(feature = "nfc-pcsc"))]
    #[test]
    fn select_reader_pcsc_without_feature_errors() {
        match select_reader(ReaderKind::Pcsc, Duration::from_millis(1)) {
            Ok(_) => panic!("expected pcsc selection to fail"),
            Err(err) => assert!(matches!(err, ReaderError::Backend { .. })),
        }
    }
}
