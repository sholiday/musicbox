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
    use super::{DisplayError, StatusDisplay};
    use crate::{
        controller::{ControllerAction, Track},
        telemetry::StatusSnapshot,
    };
    use embedded_graphics::{
        mono_font::{MonoTextStyleBuilder, ascii::FONT_9X15_BOLD},
        prelude::*,
        primitives::{PrimitiveStyle, Rectangle},
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
    use std::{
        collections::HashMap,
        io,
        io::Write,
        path::{Path, PathBuf},
        time::Duration,
    };
    use thiserror::Error;

    type BusyPin = CdevPin;
    type DcPin = CdevPin;
    type RstPin = CdevPin;
    type PwrPin = CdevPin;
    const GPIO_CONSUMER_TAG: &str = "musicbox-waveshare";
    const COVER_SIZE: u32 = 116;

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
        last_render_key: Option<RenderCacheKey>,
        metadata_cache: HashMap<PathBuf, TrackDisplayMetadata>,
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
                last_render_key: None,
                metadata_cache: HashMap::new(),
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
            self.set_window_v4(0, 0, 121, 249)?;
            self.set_cursor_v4(0, 0)?;
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

        fn render_snapshot(&mut self, snapshot: &StatusSnapshot) -> Result<(), WaveshareError> {
            let rendered = self.rendered_status(snapshot);
            if self
                .last_render_key
                .as_ref()
                .map(|prev| prev == &rendered.cache_key)
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

            if let Some(cover) = &rendered.cover {
                draw_cover(&mut frame, cover, Point::new(3, 3));
            }

            let text_x = if rendered.cover.is_some() { 124 } else { 4 };
            let max_chars = if rendered.cover.is_some() { 13 } else { 26 };
            let line_height = font.character_size.height as i32 + 2;
            let mut cursor_y = if rendered.cover.is_some() { 6 } else { 4 };

            for line in rendered.lines {
                for wrapped in wrap_display_line(&line, max_chars).into_iter().take(4) {
                    Text::with_baseline(
                        &wrapped,
                        Point::new(text_x, cursor_y),
                        style,
                        Baseline::Top,
                    )
                    .draw(&mut frame)
                    .expect("render text onto display buffer");
                    cursor_y += line_height;
                    if cursor_y > 112 {
                        break;
                    }
                }
                if cursor_y > 112 {
                    break;
                }
            }

            self.display_frame_v4(frame.buffer())?;
            self.last_render_key = Some(rendered.cache_key);
            Ok(())
        }

        fn rendered_status(&mut self, snapshot: &StatusSnapshot) -> RenderedStatus {
            let idle_line = format!("Idle polls: {}", snapshot.idle_events);
            match snapshot.last_action.as_ref() {
                Some(ControllerAction::Started { card, track }) => {
                    self.rendered_track_status("Playing", Some(card), track)
                }
                Some(ControllerAction::Switched {
                    to_card, to_track, ..
                }) => self.rendered_track_status("Playing", Some(to_card), to_track),
                Some(ControllerAction::Stopped { .. }) => RenderedStatus::without_cover(vec![
                    "Musicbox".to_string(),
                    "Stopped".to_string(),
                    idle_line,
                ]),
                None => RenderedStatus::without_cover(vec![
                    "Musicbox".to_string(),
                    "Waiting".to_string(),
                    idle_line,
                ]),
            }
        }

        fn rendered_track_status(
            &mut self,
            state: &str,
            _card: Option<&crate::controller::CardUid>,
            track: &Track,
        ) -> RenderedStatus {
            let metadata = self.metadata_for(track);
            let title = metadata
                .title
                .clone()
                .unwrap_or_else(|| display_name_for_track(track));
            let lines = vec![title, state.to_string()];
            RenderedStatus::with_track(lines, track.path(), metadata.cover.clone())
        }

        fn metadata_for(&mut self, track: &Track) -> &TrackDisplayMetadata {
            let path = track.path().to_path_buf();
            self.metadata_cache
                .entry(path)
                .or_insert_with(|| TrackDisplayMetadata::load(track.path()))
        }
    }

    impl StatusDisplay for WaveshareDisplay {
        fn update(&mut self, snapshot: &StatusSnapshot) -> Result<(), DisplayError> {
            self.render_snapshot(snapshot).map_err(DisplayError::from)
        }

        fn shutdown(&mut self) -> Result<(), DisplayError> {
            self.send_command(0x10)?;
            self.send_data(0x01)?;
            std::thread::sleep(Duration::from_millis(2000));
            self.power.set_value(0).map_err(WaveshareError::Gpio)?;
            Ok(())
        }
    }

    #[derive(Clone, Debug, Default)]
    struct TrackDisplayMetadata {
        title: Option<String>,
        cover: Option<CoverImage>,
    }

    impl TrackDisplayMetadata {
        fn load(path: &Path) -> Self {
            use id3::TagLike;

            let Ok(tag) = id3::Tag::read_from_path(path) else {
                return Self::default();
            };

            let title = tag
                .title()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(ToOwned::to_owned);
            let cover = tag
                .pictures()
                .find_map(|picture| CoverImage::from_encoded_bytes(&picture.data).ok());

            Self { title, cover }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CoverImage {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    impl CoverImage {
        fn from_encoded_bytes(bytes: &[u8]) -> Result<Self, image::ImageError> {
            use image::{GenericImageView, imageops::FilterType};

            let image = image::load_from_memory(bytes)?;
            let (width, height) = image.dimensions();
            let side = width.min(height);
            let x = (width - side) / 2;
            let y = (height - side) / 2;
            let cropped = image.crop_imm(x, y, side, side);
            let resized = cropped.resize_exact(COVER_SIZE, COVER_SIZE, FilterType::Triangle);
            let luma = resized.to_luma8();
            Ok(Self {
                width: COVER_SIZE,
                height: COVER_SIZE,
                pixels: luma.into_raw(),
            })
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RenderCacheKey {
        lines: Vec<String>,
        track_path: Option<PathBuf>,
        has_cover: bool,
    }

    struct RenderedStatus {
        cache_key: RenderCacheKey,
        lines: Vec<String>,
        cover: Option<CoverImage>,
    }

    impl RenderedStatus {
        fn with_track(lines: Vec<String>, track_path: &Path, cover: Option<CoverImage>) -> Self {
            Self {
                cache_key: RenderCacheKey {
                    lines: lines.clone(),
                    track_path: Some(track_path.to_path_buf()),
                    has_cover: cover.is_some(),
                },
                lines,
                cover,
            }
        }

        fn without_cover(lines: Vec<String>) -> Self {
            Self {
                cache_key: RenderCacheKey {
                    lines: lines.clone(),
                    track_path: None,
                    has_cover: false,
                },
                lines,
                cover: None,
            }
        }
    }

    fn display_name_for_track(track: &Track) -> String {
        track
            .path()
            .file_stem()
            .or_else(|| track.path().file_name())
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| track.path().display().to_string())
    }

    fn draw_cover(display: &mut Display2in13, cover: &CoverImage, origin: Point) {
        let bounds = Rectangle::new(origin, Size::new(cover.width, cover.height));

        for y in 0..cover.height {
            for x in 0..cover.width {
                let idx = (y * cover.width + x) as usize;
                let threshold = bayer_threshold(x, y);
                let color = if cover.pixels[idx] < threshold {
                    Color::Black
                } else {
                    Color::White
                };
                Pixel(origin + Point::new(x as i32, y as i32), color)
                    .draw(display)
                    .expect("draw cover pixel");
            }
        }

        bounds
            .into_styled(PrimitiveStyle::with_stroke(Color::Black, 1))
            .draw(display)
            .expect("draw cover border");
    }

    fn bayer_threshold(x: u32, y: u32) -> u8 {
        const BAYER_4X4: [[u8; 4]; 4] =
            [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        BAYER_4X4[(y % 4) as usize][(x % 4) as usize] * 16 + 8
    }

    fn wrap_display_line(line: &str, max_chars: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in line.split_whitespace() {
            let separator = usize::from(!current.is_empty());
            if current.chars().count() + separator + word.chars().count() <= max_chars {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            } else {
                if !current.is_empty() {
                    lines.push(current);
                    current = String::new();
                }
                if word.chars().count() <= max_chars {
                    current.push_str(word);
                } else {
                    lines.extend(split_long_word(word, max_chars));
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    fn split_long_word(word: &str, max_chars: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for ch in word.chars() {
            if current.chars().count() == max_chars {
                lines.push(current);
                current = String::new();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn track_cache_key_includes_track_path() {
            let lines = vec!["Shared title".to_string(), "Playing".to_string()];

            let first = RenderedStatus::with_track(lines.clone(), Path::new("one.mp3"), None);
            let second = RenderedStatus::with_track(lines, Path::new("two.mp3"), None);

            assert_ne!(first.cache_key, second.cache_key);
        }

        #[test]
        fn track_cache_key_includes_cover_presence() {
            let lines = vec!["Shared title".to_string(), "Playing".to_string()];
            let cover = CoverImage {
                width: 1,
                height: 1,
                pixels: vec![0],
            };

            let without_cover =
                RenderedStatus::with_track(lines.clone(), Path::new("song.mp3"), None);
            let with_cover = RenderedStatus::with_track(lines, Path::new("song.mp3"), Some(cover));

            assert_ne!(without_cover.cache_key, with_cover.cache_key);
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
