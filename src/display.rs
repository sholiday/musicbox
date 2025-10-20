use crate::{
    controller::{CardUid, ControllerAction, Track},
    telemetry::StatusSnapshot,
};
use std::time::{Duration, SystemTime};
use thiserror::Error;

/// Errors that can occur while interacting with a status display backend.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DisplayError {
    #[error("display backend is not available")]
    BackendUnavailable,
    #[cfg(feature = "waveshare-display")]
    #[error(transparent)]
    Waveshare(#[from] waveshare::WaveshareError),
}

/// Render the latest controller status to an external display.
pub trait StatusDisplay: Send {
    fn update(&mut self, snapshot: &StatusSnapshot) -> Result<(), DisplayError>;

    fn shutdown(&mut self) -> Result<(), DisplayError> {
        Ok(())
    }
}

/// A no-op display backend used when no hardware is configured.
#[derive(Debug, Default)]
pub struct NullDisplay;

impl StatusDisplay for NullDisplay {
    fn update(&mut self, _snapshot: &StatusSnapshot) -> Result<(), DisplayError> {
        Ok(())
    }
}

/// Returns human-readable status lines describing the current controller state.
pub fn status_lines(snapshot: &StatusSnapshot) -> Vec<String> {
    let idle_line = format!("Idle polls: {}", snapshot.idle_events);

    let (state, active_card, active_track) = match snapshot.last_action.as_ref() {
        Some(ControllerAction::Started { card, track }) => {
            ("Playing".to_string(), Some(card), Some(track))
        }
        Some(ControllerAction::Switched {
            to_card, to_track, ..
        }) => ("Switched".to_string(), Some(to_card), Some(to_track)),
        Some(ControllerAction::Stopped { .. }) => ("Stopped".to_string(), None, None),
        None => ("Waiting".to_string(), None, None),
    };

    let card_line = format!("Card: {}", format_card(active_card));
    let track_line = format!("Track: {}", format_track(active_track));

    let updated_line = snapshot
        .last_update
        .and_then(|instant| SystemTime::now().duration_since(instant).ok())
        .map(format_update_age)
        .unwrap_or_else(|| "Updated: –".to_string());

    vec![
        "Musicbox".to_string(),
        format!("State: {state}"),
        idle_line,
        card_line,
        track_line,
        updated_line,
    ]
}

fn format_card(card: Option<&CardUid>) -> String {
    card.map(|uid| uid.to_hex_lowercase())
        .unwrap_or_else(|| "–".to_string())
}

fn format_track(track: Option<&Track>) -> String {
    match track {
        Some(track) => track
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| track.path().display().to_string()),
        None => "–".to_string(),
    }
}

fn format_update_age(delta: Duration) -> String {
    if delta.as_secs() == 0 {
        "Updated: just now".to_string()
    } else {
        format!("Updated: {}s ago", delta.as_secs())
    }
}

#[cfg(all(feature = "waveshare-display", target_os = "linux"))]
pub mod waveshare {
    use super::{DisplayError, StatusDisplay, status_lines};
    use crate::telemetry::StatusSnapshot;
    use embedded_graphics::{
        mono_font::{MonoTextStyleBuilder, ascii::FONT_6X12},
        prelude::*,
        text::{Baseline, Text},
    };
    use epd_waveshare::{
        epd2in13_v2::{Display2in13, Epd2in13},
        prelude::{Color, DisplayRotation, WaveshareDisplay as EpdDriver},
    };
    use gpio_cdev::{Chip, LineRequestFlags};
    use linux_embedded_hal::{
        CdevPin, Delay, SpidevDevice,
        spidev::{SpiModeFlags, Spidev, SpidevOptions},
    };
    use std::{io, path::Path};
    use thiserror::Error;

    type BusyPin = CdevPin;
    type DcPin = CdevPin;
    type RstPin = CdevPin;
    const GPIO_CONSUMER_TAG: &str = "musicbox-waveshare";

    fn open_spi(path: &Path, speed_hz: u32) -> Result<SpidevDevice, io::Error> {
        let mut spi = Spidev::open(path)?;
        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(speed_hz)
            .mode(SpiModeFlags::SPI_MODE_0)
            .build();
        spi.configure(&options)?;
        Ok(SpidevDevice(spi))
    }

    fn to_line_offset(pin: u64) -> Result<u32, WaveshareError> {
        u32::try_from(pin).map_err(|_| WaveshareError::PinOutOfRange(pin))
    }

    fn request_input_pin(
        chip: &mut Chip,
        offset: u32,
    ) -> Result<CdevPin, gpio_cdev::errors::Error> {
        let line = chip.get_line(offset)?;
        let handle = line.request(LineRequestFlags::INPUT, 0, GPIO_CONSUMER_TAG)?;
        CdevPin::new(handle)
    }

    fn request_output_pin(
        chip: &mut Chip,
        offset: u32,
        initial_high: bool,
    ) -> Result<CdevPin, gpio_cdev::errors::Error> {
        let line = chip.get_line(offset)?;
        let initial_value = if initial_high { 1 } else { 0 };
        let handle = line.request(LineRequestFlags::OUTPUT, initial_value, GPIO_CONSUMER_TAG)?;
        CdevPin::new(handle)
    }

    /// Configuration for the Waveshare E-Ink HAT wiring and SPI bus.
    #[derive(Clone)]
    pub struct WaveshareConfig {
        pub spi_path: String,
        pub busy_pin: u64,
        pub dc_pin: u64,
        pub reset_pin: u64,
        pub spi_speed_hz: u32,
        pub rotation: DisplayRotation,
        pub gpio_chip_path: String,
    }

    impl Default for WaveshareConfig {
        fn default() -> Self {
            Self {
                spi_path: "/dev/spidev0.0".to_string(),
                busy_pin: 24,
                dc_pin: 25,
                reset_pin: 17,
                spi_speed_hz: 8_000_000,
                rotation: DisplayRotation::Rotate270,
                gpio_chip_path: "/dev/gpiochip0".to_string(),
            }
        }
    }

    /// Errors from initializing or updating the Waveshare display.
    #[derive(Debug, Error)]
    pub enum WaveshareError {
        #[error("SPI error: {0}")]
        Spi(#[from] io::Error),
        #[error("GPIO error: {0}")]
        Gpio(#[from] gpio_cdev::errors::Error),
        #[error("GPIO pin {0} is out of range for this platform")]
        PinOutOfRange(u64),
        #[error("display driver error: {0}")]
        Driver(String),
    }

    /// Renderer that targets the Waveshare 2.13\" e-ink HAT.
    pub struct WaveshareDisplay {
        spi: SpidevDevice,
        epd: Epd2in13<SpidevDevice, BusyPin, DcPin, RstPin, Delay>,
        delay: Delay,
        rotation: DisplayRotation,
    }

    impl WaveshareDisplay {
        pub fn new(config: WaveshareConfig) -> Result<Self, WaveshareError> {
            let spi_path = Path::new(&config.spi_path);
            if !spi_path.exists() {
                return Err(WaveshareError::Spi(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("SPI device {} not found", config.spi_path),
                )));
            }

            let mut spi = open_spi(spi_path, config.spi_speed_hz)?;

            let mut chip =
                Chip::new(Path::new(&config.gpio_chip_path)).map_err(WaveshareError::Gpio)?;
            let busy_offset = to_line_offset(config.busy_pin)?;
            let dc_offset = to_line_offset(config.dc_pin)?;
            let rst_offset = to_line_offset(config.reset_pin)?;

            let busy = request_input_pin(&mut chip, busy_offset).map_err(WaveshareError::Gpio)?;
            let dc =
                request_output_pin(&mut chip, dc_offset, false).map_err(WaveshareError::Gpio)?;
            let rst =
                request_output_pin(&mut chip, rst_offset, true).map_err(WaveshareError::Gpio)?;

            let mut delay = Delay;
            let mut epd = Epd2in13::new(&mut spi, busy, dc, rst, &mut delay, None)
                .map_err(|err| driver_error(err))?;

            epd.clear_frame(&mut spi, &mut delay)
                .map_err(|err| driver_error(err))?;
            epd.display_frame(&mut spi, &mut delay)
                .map_err(|err| driver_error(err))?;

            Ok(Self {
                spi,
                epd,
                delay,
                rotation: config.rotation,
            })
        }

        fn render_lines(&mut self, lines: &[String]) -> Result<(), WaveshareError> {
            let mut frame = Display2in13::default();
            frame.set_rotation(self.rotation);
            let _ = frame.clear(Color::White);

            let style = MonoTextStyleBuilder::new()
                .font(&FONT_6X12)
                .text_color(Color::Black)
                .background_color(Color::White)
                .build();

            let mut cursor_y = 10;
            for line in lines {
                let display_line: String = line.chars().take(40).collect();
                Text::with_baseline(&display_line, Point::new(4, cursor_y), style, Baseline::Top)
                    .draw(&mut frame)
                    .expect("render text onto display buffer");
                cursor_y += 14;
            }

            self.epd
                .update_frame(&mut self.spi, frame.buffer(), &mut self.delay)
                .map_err(|err| driver_error(err))?;
            self.epd
                .display_frame(&mut self.spi, &mut self.delay)
                .map_err(|err| driver_error(err))?;
            Ok(())
        }
    }

    impl StatusDisplay for WaveshareDisplay {
        fn update(&mut self, snapshot: &StatusSnapshot) -> Result<(), DisplayError> {
            let lines = status_lines(snapshot);
            self.render_lines(&lines).map_err(DisplayError::from)
        }

        fn shutdown(&mut self) -> Result<(), DisplayError> {
            self.epd
                .sleep(&mut self.spi, &mut self.delay)
                .map_err(|err| driver_error(err))?;
            Ok(())
        }
    }

    fn driver_error<E: std::fmt::Display>(err: E) -> WaveshareError {
        WaveshareError::Driver(err.to_string())
    }
}

#[cfg(all(feature = "waveshare-display", not(target_os = "linux")))]
pub mod waveshare {
    use super::{DisplayError, StatusDisplay};
    use crate::telemetry::StatusSnapshot;
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("waveshare display is only supported on Linux targets")]
    pub struct WaveshareError;

    #[derive(Debug, Clone, Default)]
    pub struct WaveshareConfig;

    pub struct WaveshareDisplay;

    impl WaveshareDisplay {
        pub fn new(_: WaveshareConfig) -> Result<Self, WaveshareError> {
            Err(WaveshareError)
        }
    }

    impl StatusDisplay for WaveshareDisplay {
        fn update(&mut self, _snapshot: &StatusSnapshot) -> Result<(), DisplayError> {
            Err(DisplayError::BackendUnavailable)
        }
    }
}
