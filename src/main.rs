use clap::{Args, Parser, Subcommand, ValueEnum, builder::ValueHint};
use musicbox::app::{RunLoopError, controller_from_config_path, run_until_shutdown};
use musicbox::audio::RodioPlayer;
use musicbox::config::{self, ConfigEditError};
use musicbox::controller::{AudioPlayer, CardUid, CardUidParseError, PlayerError, Track};
#[cfg(feature = "waveshare-display")]
use musicbox::display;
#[cfg(feature = "waveshare-display")]
use musicbox::display::waveshare::{WaveshareConfig, WaveshareDisplay};
use musicbox::reader::{NfcReader, ReaderError, ReaderEvent};
use musicbox::telemetry::{self, SharedStatus};
#[cfg(feature = "debug-http")]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(feature = "waveshare-display")]
type SharedStatusDisplay = Arc<Mutex<Box<dyn display::StatusDisplay>>>;

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

    #[cfg(feature = "waveshare-display")]
    #[command(flatten)]
    waveshare: WaveshareDisplayArgs,

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
    Add(TagAddArgs),
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
    config: Option<PathBuf>,

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

#[cfg(feature = "waveshare-display")]
#[derive(Debug, Args, Clone)]
struct WaveshareDisplayArgs {
    #[arg(long, help = "Enable status updates on a Waveshare e-ink Pi HAT")]
    waveshare_display: bool,

    #[arg(
        long = "waveshare-spi",
        value_name = "PATH",
        default_value = "/dev/spidev0.0",
        value_hint = ValueHint::FilePath,
        help = "SPI device path for the display"
    )]
    spi_path: String,

    #[arg(
        long = "waveshare-busy-pin",
        value_name = "PIN",
        default_value_t = 24,
        help = "GPIO pin wired to the display BUSY line"
    )]
    busy_pin: u64,

    #[arg(
        long = "waveshare-dc-pin",
        value_name = "PIN",
        default_value_t = 25,
        help = "GPIO pin wired to the display D/C line"
    )]
    dc_pin: u64,

    #[arg(
        long = "waveshare-reset-pin",
        value_name = "PIN",
        default_value_t = 17,
        help = "GPIO pin wired to the display RESET line"
    )]
    reset_pin: u64,

    #[arg(
        long = "waveshare-gpio-chip",
        value_name = "PATH",
        default_value = "/dev/gpiochip0",
        value_hint = ValueHint::FilePath,
        help = "GPIO character device path providing the configured pins"
    )]
    gpio_chip_path: String,
}

#[cfg(feature = "waveshare-display")]
fn waveshare_config_from_args(args: &WaveshareDisplayArgs) -> Option<WaveshareConfig> {
    if !args.waveshare_display {
        return None;
    }

    let mut config = WaveshareConfig::default();
    config.spi_path = args.spi_path.clone();
    config.busy_pin = args.busy_pin;
    config.dc_pin = args.dc_pin;
    config.reset_pin = args.reset_pin;
    config.gpio_chip_path = args.gpio_chip_path.clone();
    Some(config)
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ReaderKind {
    Auto,
    Pcsc,
    Noop,
}

#[derive(Debug, Error)]
enum TagError {
    #[error(
        "configuration path required; pass --config <PATH> or provide CONFIG before the command"
    )]
    MissingConfig,
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

/// Parses command-line arguments and calls the appropriate handler.
fn run() -> Result<(), RunError> {
    let cli = Cli::parse();

    let Cli {
        config,
        poll_interval_ms,
        reader,
        silent,
        #[cfg(feature = "waveshare-display")]
        waveshare,
        #[cfg(feature = "debug-http")]
        debug_http,
        command,
    } = cli;

    match command {
        Some(Command::Tag(tag_command)) => {
            handle_tag_command(tag_command, config.clone(), reader, poll_interval_ms)?;
        }
        Some(Command::Manual(manual_command)) => {
            handle_manual_command(manual_command, silent)?;
        }
        Some(Command::Add(args)) => {
            handle_tag_add(args, config.clone(), reader, poll_interval_ms)?;
        }
        None => {
            let config_path = config.ok_or(RunError::MissingConfig)?;
            #[cfg(feature = "waveshare-display")]
            let waveshare_config = waveshare_config_from_args(&waveshare);
            run_player_main(
                config_path,
                poll_interval_ms,
                reader,
                silent,
                #[cfg(feature = "waveshare-display")]
                waveshare_config,
                #[cfg(feature = "debug-http")]
                debug_http,
            )?;
        }
    }

    Ok(())
}

/// The main entry point for running the music player.
fn run_player_main(
    config_path: PathBuf,
    poll_interval_ms: u64,
    reader_kind: ReaderKind,
    silent: bool,
    #[cfg(feature = "waveshare-display")] waveshare_config: Option<WaveshareConfig>,
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

    let controller = Arc::new(Mutex::new(controller_from_config_path(
        &config_path,
        player,
    )?));
    let poll_duration = Duration::from_millis(poll_interval_ms);
    let mut reader = select_reader(reader_kind, poll_duration)?.into_reader();

    let status = SharedStatus::default();
    let action_status_state = status.clone();
    let idle_status_state = status.clone();

    #[cfg(feature = "debug-http")]
    if let Some(addr) = debug_http {
        let server_status = status.clone();
        let server_controller = controller.clone();
        let server_config = config_path.clone();
        std::thread::spawn(move || {
            let state = musicbox::web::DebugState {
                status: server_status,
                controller: server_controller,
                config_path: server_config,
            };
            if let Err(err) = musicbox::web::serve(state, addr) {
                tracing::error!(?err, "debug server terminated");
            }
        });
    }

    #[cfg(feature = "waveshare-display")]
    let display: Option<SharedStatusDisplay> =
        waveshare_config.and_then(|config| match WaveshareDisplay::new(config) {
            Ok(device) => {
                println!("Waveshare display connected; status updates enabled.");
                Some(Arc::new(Mutex::new(
                    Box::new(device) as Box<dyn display::StatusDisplay>
                )))
            }
            Err(err) => {
                eprintln!("Failed to initialize Waveshare display: {err}");
                tracing::warn!(?err, "waveshare display initialization failed");
                None
            }
        });

    #[cfg(feature = "waveshare-display")]
    if let Some(handle) = &display {
        match handle.lock() {
            Ok(mut device) => {
                if let Err(err) = device.update(&status.snapshot()) {
                    tracing::warn!(?err, "initial waveshare display update failed");
                }
            }
            Err(err) => {
                tracing::warn!(?err, "waveshare display mutex poisoned during init");
            }
        }
    }

    println!("Loaded configuration from {}", config_path.display());
    println!("Awaiting NFC interactions (reader not connected in this environment).");

    let sleep_duration = Duration::from_millis(poll_interval_ms);

    #[cfg(feature = "waveshare-display")]
    let display_for_actions = display.clone();
    #[cfg(feature = "waveshare-display")]
    let display_for_idle = display.clone();

    run_until_shutdown(
        controller.clone(),
        &mut reader,
        {
            #[cfg(feature = "waveshare-display")]
            let display_for_actions = display_for_actions;
            let action_status = action_status_state;
            move |action| {
                println!("Controller action: {:?}", action);
                action_status.record_action(action.clone());
                tracing::info!(?action, "controller action");
                #[cfg(feature = "waveshare-display")]
                {
                    if let Some(handle) = &display_for_actions {
                        let snapshot = action_status.snapshot();
                        match handle.lock() {
                            Ok(mut device) => {
                                if let Err(err) = device.update(&snapshot) {
                                    tracing::warn!(?err, "waveshare display update failed");
                                }
                            }
                            Err(err) => {
                                tracing::warn!(?err, "waveshare display mutex poisoned");
                            }
                        }
                    }
                }
            }
        },
        {
            #[cfg(feature = "waveshare-display")]
            let display_for_idle = display_for_idle;
            let idle_status = idle_status_state;
            move || {
                idle_status.record_idle();
                #[cfg(feature = "waveshare-display")]
                {
                    if let Some(handle) = &display_for_idle {
                        let snapshot = idle_status.snapshot();
                        if snapshot.idle_events % 100 == 0 {
                            match handle.lock() {
                                Ok(mut device) => {
                                    if let Err(err) = device.update(&snapshot) {
                                        tracing::warn!(?err, "waveshare display update failed");
                                    }
                                }
                                Err(err) => {
                                    tracing::warn!(?err, "waveshare display mutex poisoned");
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(sleep_duration);
            }
        },
    )?;

    #[cfg(feature = "waveshare-display")]
    if let Some(handle) = &display {
        match handle.lock() {
            Ok(mut device) => {
                if let Err(err) = device.shutdown() {
                    tracing::warn!(?err, "failed to put waveshare display to sleep");
                }
            }
            Err(err) => {
                tracing::warn!(?err, "waveshare display mutex poisoned during shutdown");
            }
        }
    }

    println!("Reader requested shutdown. Exiting.");
    tracing::info!(snapshot = ?status.snapshot(), "final status");

    Ok(())
}

/// Handles the `tag` subcommand.
fn handle_tag_command(
    command: TagCommand,
    inherited_config: Option<PathBuf>,
    default_reader: ReaderKind,
    default_poll_ms: u64,
) -> Result<(), TagError> {
    match command {
        TagCommand::Add(args) => {
            handle_tag_add(args, inherited_config, default_reader, default_poll_ms)
        }
    }
}

/// Handles the `tag add` subcommand.
fn handle_tag_add(
    args: TagAddArgs,
    inherited_config: Option<PathBuf>,
    default_reader: ReaderKind,
    default_poll_ms: u64,
) -> Result<(), TagError> {
    let TagAddArgs {
        config,
        track,
        card,
        reader,
        poll_interval_ms,
        skip_tag_write,
    } = args;

    let config_path = config.or(inherited_config).ok_or(TagError::MissingConfig)?;
    let reader_kind = reader.unwrap_or(default_reader);
    let poll_ms = poll_interval_ms.unwrap_or(default_poll_ms);
    let poll_duration = Duration::from_millis(poll_ms);

    let track_str = path_to_string(&track)?;

    let mut effective_reader_kind = reader_kind;
    let mut auto_generated_uid = false;

    let uid = if let Some(card_hex) = card {
        CardUid::from_hex(card_hex.trim())?
    } else if matches!(reader_kind, ReaderKind::Noop) {
        auto_generated_uid = true;
        generate_synthetic_card_uid()
    } else {
        let selection = select_reader(reader_kind, poll_duration)?;
        effective_reader_kind = selection.kind();
        if matches!(effective_reader_kind, ReaderKind::Noop) {
            auto_generated_uid = true;
            generate_synthetic_card_uid()
        } else {
            acquire_card_uid(selection.into_reader())?
        }
    };

    if auto_generated_uid {
        println!(
            "Generated synthetic card UID {} because the selected reader cannot scan cards.",
            uid
        );
    }

    config::add_card_to_config(&config_path, &uid, &track_str)?;

    println!(
        "Mapped card {} to {} in {}",
        uid,
        track_str,
        config_path.display()
    );

    if skip_tag_write {
        println!("Skipping NFC tag write (per --skip-tag-write).");
    } else if let Err(err) =
        attempt_tag_write(effective_reader_kind, poll_duration, &uid, &track_str)
    {
        tracing::warn!(?err, "failed to write NFC tag; config still updated");
    }

    Ok(())
}

/// Converts a `Path` to a `String`.
fn path_to_string(path: &Path) -> Result<String, TagError> {
    path.to_str()
        .map(|s| s.to_owned())
        .ok_or_else(|| TagError::InvalidTrackPath(path.to_path_buf()))
}

fn generate_synthetic_card_uid() -> CardUid {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);

    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(&now.as_secs().to_be_bytes());
    bytes.extend_from_slice(&now.subsec_nanos().to_be_bytes());

    CardUid::new(bytes)
}

/// Waits for a card to be presented to the reader and returns its UID.
fn acquire_card_uid(mut reader: Box<dyn NfcReader>) -> Result<CardUid, TagError> {
    loop {
        match reader.next_event()? {
            ReaderEvent::CardPresent { uid } => return Ok(uid),
            ReaderEvent::Idle => continue,
            ReaderEvent::Shutdown => return Err(TagError::ReaderShutdown),
        }
    }
}

/// Attempts to write the track metadata to the NFC tag.
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

/// Handles the `manual` subcommand.
fn handle_manual_command(command: ManualCommand, silent: bool) -> Result<(), RunError> {
    match command {
        ManualCommand::Trigger(args) => handle_manual_trigger(args, silent),
    }
}

/// Handles the `manual trigger` subcommand.
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

struct ReaderSelection {
    reader: Option<Box<dyn NfcReader>>,
    effective_kind: ReaderKind,
}

impl ReaderSelection {
    fn new(effective_kind: ReaderKind, reader: Box<dyn NfcReader>) -> Self {
        Self {
            reader: Some(reader),
            effective_kind,
        }
    }

    fn noop() -> Self {
        Self::new(ReaderKind::Noop, Box::new(NoopReader::default()))
    }

    fn into_reader(self) -> Box<dyn NfcReader> {
        self.reader.expect("reader already taken")
    }

    fn kind(&self) -> ReaderKind {
        self.effective_kind
    }
}

fn select_reader(kind: ReaderKind, poll: Duration) -> Result<ReaderSelection, ReaderError> {
    match kind {
        ReaderKind::Noop => Ok(ReaderSelection::noop()),
        ReaderKind::Pcsc => {
            build_pcsc_reader(poll).map(|reader| ReaderSelection::new(ReaderKind::Pcsc, reader))
        }
        ReaderKind::Auto => match build_pcsc_reader(poll) {
            Ok(reader) => Ok(ReaderSelection::new(ReaderKind::Pcsc, reader)),
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "PC/SC reader unavailable; falling back to noop reader"
                );
                Ok(ReaderSelection::noop())
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
        let selection = select_reader(ReaderKind::Noop, Duration::from_millis(1)).unwrap();
        let mut reader = selection.into_reader();
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
