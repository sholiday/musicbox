use crate::controller::CardUid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ReaderError {
    #[error("reader backend error: {message}")]
    Backend { message: String },
    #[error("card response status {sw1:02X}{sw2:02X}")]
    StatusWord { sw1: u8, sw2: u8 },
    #[cfg(feature = "nfc-pcsc")]
    #[error("pcsc error: {0}")]
    Pcsc(#[from] pcsc::Error),
}

impl ReaderError {
    pub fn backend(message: impl Into<String>) -> Self {
        ReaderError::Backend {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderEvent {
    CardPresent { uid: CardUid },
    Idle,
    Shutdown,
}

/// An interface for reading events from an NFC reader.
pub trait NfcReader {
    fn next_event(&mut self) -> Result<ReaderEvent, ReaderError>;
}

impl<T: NfcReader + ?Sized> NfcReader for Box<T> {
    fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
        (**self).next_event()
    }
}

#[cfg(feature = "nfc-pcsc")]
pub mod pcsc_backend {
    use super::{CardUid, NfcReader, ReaderError, ReaderEvent};
    use pcsc::{Card, Context, Error as PcscError, Protocols, Scope, ShareMode, Status};
    use std::time::Duration;

    /// A `NfcReader` that uses the `pcsc` crate to communicate with a PC/SC reader.
    pub struct PcscReader {
        context: Context,
        card: Option<Card>,
        poll_interval: Duration,
        last_uid: Option<CardUid>,
    }

    impl PcscReader {
        pub fn new(poll_interval: Duration) -> Result<Self, ReaderError> {
            let context = Context::establish(Scope::User).map_err(ReaderError::from)?;
            Ok(Self {
                context,
                card: None,
                poll_interval,
                last_uid: None,
            })
        }

        fn connect_card(&mut self) -> Result<(), ReaderError> {
            let readers_buf = self
                .context
                .list_readers_owned()
                .map_err(ReaderError::from)?;
            if readers_buf.is_empty() {
                return Err(ReaderError::backend("no PC/SC readers available"));
            }
            let reader_name = readers_buf
                .first()
                .ok_or_else(|| ReaderError::backend("failed to obtain reader name from PC/SC"))?
                .as_c_str();
            match self
                .context
                .connect(reader_name, ShareMode::Shared, Protocols::ANY)
            {
                Ok(card) => {
                    self.card = Some(card);
                    self.last_uid = None;
                }
                Err(PcscError::NoSmartcard) => {
                    // Card absent: keep polling until one is presented.
                    self.card = None;
                    self.last_uid = None;
                }
                Err(err) => return Err(ReaderError::from(err)),
            }
            Ok(())
        }

        fn read_uid(card: &Card) -> Result<CardUid, ReaderError> {
            const SEND_BUFFER: [u8; 5] = [0xFF, 0xCA, 0x00, 0x00, 0x00];
            let mut recv_buffer = [0u8; pcsc::MAX_BUFFER_SIZE];
            let response = card
                .transmit(&SEND_BUFFER, &mut recv_buffer)
                .map_err(ReaderError::from)?;
            if response.len() < 2 {
                return Err(ReaderError::backend("card UID response too short"));
            }
            let (data, status) = response.split_at(response.len() - 2);
            if status != [0x90, 0x00] {
                return Err(ReaderError::StatusWord {
                    sw1: status[0],
                    sw2: status[1],
                });
            }
            Ok(CardUid::new(data.to_vec()))
        }

        fn poll(&mut self) -> Result<Option<ReaderEvent>, ReaderError> {
            if self.card.is_none() {
                self.connect_card()?;
            }

            if self.card.is_none() {
                return Ok(None);
            }

            let status = {
                let card = self
                    .card
                    .as_ref()
                    .expect("card present after successful connect");
                match card.status2_owned().map_err(ReaderError::from) {
                    Ok(status) => status,
                    Err(ReaderError::Pcsc(PcscError::RemovedCard | PcscError::ResetCard)) => {
                        self.card = None;
                        self.last_uid = None;
                        return Ok(None);
                    }
                    Err(err) => return Err(err),
                }
            };

            if !status.status().contains(Status::PRESENT) {
                self.card = None;
                self.last_uid = None;
                return Ok(None);
            }

            let uid_result = {
                let card = self.card.as_ref().expect("card present while decoding UID");
                Self::read_uid(card)
            };

            match uid_result {
                Ok(uid) => match self.last_uid.as_ref() {
                    Some(previous) if previous == &uid => Ok(Some(ReaderEvent::Idle)),
                    _ => {
                        self.last_uid = Some(uid.clone());
                        Ok(Some(ReaderEvent::CardPresent { uid }))
                    }
                },
                Err(ReaderError::StatusWord { sw1: 0x63, sw2: 0x00 }) => {
                    tracing::debug!("PC/SC reported status 6300; resetting reader state");
                    self.card = None;
                    self.last_uid = None;
                    Ok(None)
                }
                Err(err)
                    if matches!(
                        err,
                        ReaderError::Pcsc(PcscError::RemovedCard | PcscError::ResetCard)
                    ) =>
                {
                    self.card = None;
                    self.last_uid = None;
                    Ok(None)
                }
                Err(err) => Err(err),
            }
        }
    }

    impl NfcReader for PcscReader {
        fn next_event(&mut self) -> Result<ReaderEvent, ReaderError> {
            loop {
                match self.poll()? {
                    Some(event) => return Ok(event),
                    None => std::thread::sleep(self.poll_interval),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_error_backend_helper_builds_variant() {
        let err = ReaderError::backend("test");
        assert_eq!(
            err,
            ReaderError::Backend {
                message: "test".into()
            }
        );
    }

    #[test]
    fn reader_event_card_present_holds_uid() {
        let uid = CardUid::new(vec![1, 2, 3, 4]);
        let event = ReaderEvent::CardPresent { uid: uid.clone() };
        assert_eq!(event, ReaderEvent::CardPresent { uid });
    }
}
