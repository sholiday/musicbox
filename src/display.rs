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
        mono_font::{MonoTextStyleBuilder, ascii::FONT_9X15_BOLD},
        prelude::*,
        text::{Baseline, Text},
    };
    use epd_waveshare::{
        epd2in13_v2::Display2in13,
        prelude::{Color, DisplayRotation},
    };
    use gpio_cdev::{Chip, LineRequestFlags};
    use linux_embedded_hal::{
        CdevPin, SpidevDevice,
        spidev::{SpiModeFlags, Spidev, SpidevOptions},
    };
    use std::{io, io::Write, path::Path, time::Duration};
    use thiserror::Error;

    type BusyPin = CdevPin;
    type DcPin = CdevPin;
    type RstPin = CdevPin;
    type PwrPin = CdevPin;
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
        pub power_pin: u64,
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
                power_pin: 18,
                spi_speed_hz: 4_000_000,
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
        busy: BusyPin,
        dc: DcPin,
        reset: RstPin,
        power: PwrPin,
        rotation: DisplayRotation,
        last_lines: Option<Vec<String>>,
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

            let spi = open_spi(spi_path, config.spi_speed_hz)?;

            let mut chip =
                Chip::new(Path::new(&config.gpio_chip_path)).map_err(WaveshareError::Gpio)?;
            let busy_offset = to_line_offset(config.busy_pin)?;
            let dc_offset = to_line_offset(config.dc_pin)?;
            let rst_offset = to_line_offset(config.reset_pin)?;
            let pwr_offset = to_line_offset(config.power_pin)?;

            let busy = request_input_pin(&mut chip, busy_offset).map_err(WaveshareError::Gpio)?;
            let dc =
                request_output_pin(&mut chip, dc_offset, false).map_err(WaveshareError::Gpio)?;
            let rst =
                request_output_pin(&mut chip, rst_offset, true).map_err(WaveshareError::Gpio)?;
            let power =
                request_output_pin(&mut chip, pwr_offset, true).map_err(WaveshareError::Gpio)?;

            let mut display = Self {
                spi,
                busy,
                dc,
                reset: rst,
                power,
                rotation: config.rotation,
                last_lines: None,
            };
            display.init_v4()?;
            display.clear_v4(Color::White)?;
            Ok(display)
        }

        fn reset_v4(&mut self) -> Result<(), WaveshareError> {
            self.reset.set_value(1).map_err(WaveshareError::Gpio)?;
            std::thread::sleep(Duration::from_millis(20));
            self.reset.set_value(0).map_err(WaveshareError::Gpio)?;
            std::thread::sleep(Duration::from_millis(2));
            self.reset.set_value(1).map_err(WaveshareError::Gpio)?;
            std::thread::sleep(Duration::from_millis(20));
            Ok(())
        }

        fn wait_until_idle_v4(&mut self) -> Result<(), WaveshareError> {
            while self.busy.get_value().map_err(WaveshareError::Gpio)? == 1 {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        }

        fn send_command(&mut self, command: u8) -> Result<(), WaveshareError> {
            self.dc.set_value(0).map_err(WaveshareError::Gpio)?;
            self.spi.write_all(&[command]).map_err(WaveshareError::Spi)
        }

        fn send_data(&mut self, data: u8) -> Result<(), WaveshareError> {
            self.dc.set_value(1).map_err(WaveshareError::Gpio)?;
            self.spi.write_all(&[data]).map_err(WaveshareError::Spi)
        }

        fn send_data_slice(&mut self, data: &[u8]) -> Result<(), WaveshareError> {
            self.dc.set_value(1).map_err(WaveshareError::Gpio)?;
            self.spi.write_all(data).map_err(WaveshareError::Spi)
        }

        fn set_window_v4(
            &mut self,
            x_start: u16,
            y_start: u16,
            x_end: u16,
            y_end: u16,
        ) -> Result<(), WaveshareError> {
            self.send_command(0x44)?;
            self.send_data(((x_start >> 3) & 0xff) as u8)?;
            self.send_data(((x_end >> 3) & 0xff) as u8)?;

            self.send_command(0x45)?;
            self.send_data((y_start & 0xff) as u8)?;
            self.send_data((y_start >> 8) as u8)?;
            self.send_data((y_end & 0xff) as u8)?;
            self.send_data((y_end >> 8) as u8)?;
            Ok(())
        }

        fn set_cursor_v4(&mut self, x: u16, y: u16) -> Result<(), WaveshareError> {
            self.send_command(0x4e)?;
            self.send_data((x & 0xff) as u8)?;

            self.send_command(0x4f)?;
            self.send_data((y & 0xff) as u8)?;
            self.send_data((y >> 8) as u8)?;
            Ok(())
        }

        fn init_v4(&mut self) -> Result<(), WaveshareError> {
            self.power.set_value(1).map_err(WaveshareError::Gpio)?;
            self.reset_v4()?;

            self.wait_until_idle_v4()?;
            self.send_command(0x12)?;
            self.wait_until_idle_v4()?;

            self.send_command(0x01)?;
            self.send_data(0xf9)?;
            self.send_data(0x00)?;
            self.send_data(0x00)?;

            self.send_command(0x11)?;
            self.send_data(0x03)?;

            self.set_window_v4(0, 0, 121, 249)?;
            self.set_cursor_v4(0, 0)?;

            self.send_command(0x3c)?;
            self.send_data(0x05)?;

            self.send_command(0x21)?;
            self.send_data(0x00)?;
            self.send_data(0x80)?;

            self.send_command(0x18)?;
            self.send_data(0x80)?;

            self.wait_until_idle_v4()
        }

        fn turn_on_display_v4(&mut self) -> Result<(), WaveshareError> {
            self.send_command(0x22)?;
            self.send_data(0xf7)?;
            self.send_command(0x20)?;
            self.wait_until_idle_v4()
        }

        fn display_frame_v4(&mut self, buffer: &[u8]) -> Result<(), WaveshareError> {
            self.send_command(0x24)?;
            self.send_data_slice(buffer)?;
            self.turn_on_display_v4()
        }

        fn clear_v4(&mut self, color: Color) -> Result<(), WaveshareError> {
            let byte = match color {
                Color::White => 0xff,
                Color::Black => 0x00,
            };
            let buffer = [byte; 4000];
            self.display_frame_v4(&buffer)
        }

        fn render_lines(&mut self, lines: &[String]) -> Result<(), WaveshareError> {
            if self
                .last_lines
                .as_ref()
                .map(|prev| prev == lines)
                .unwrap_or(false)
            {
                return Ok(());
            }

            let mut frame = Display2in13::default();
            frame.set_rotation(self.rotation);
            let _ = frame.clear(Color::White);

            let font = FONT_9X15_BOLD;
            let style = MonoTextStyleBuilder::new()
                .font(&font)
                .text_color(Color::Black)
                .background_color(Color::White)
                .build();

            let max_chars = 26;
            let line_height = font.character_size.height as i32 + 2;
            let mut cursor_y = 4;
            let left_margin = 4;

            for line in lines {
                let display_line: String = line.chars().take(max_chars).collect();
                Text::with_baseline(
                    &display_line,
                    Point::new(left_margin, cursor_y),
                    style,
                    Baseline::Top,
                )
                .draw(&mut frame)
                .expect("render text onto display buffer");
                cursor_y += line_height;
            }

            self.display_frame_v4(frame.buffer())?;
            self.last_lines = Some(lines.to_vec());
            Ok(())
        }
    }

    impl StatusDisplay for WaveshareDisplay {
        fn update(&mut self, snapshot: &StatusSnapshot) -> Result<(), DisplayError> {
            let lines = status_lines(snapshot);
            self.render_lines(&lines).map_err(DisplayError::from)
        }

        fn shutdown(&mut self) -> Result<(), DisplayError> {
            self.send_command(0x10)?;
            self.send_data(0x01)?;
            std::thread::sleep(Duration::from_millis(2000));
            self.power.set_value(0).map_err(WaveshareError::Gpio)?;
            Ok(())
        }
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
