use crate::controller::CardUid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ReaderError {
    #[error("reader backend error: {message}")]
    Backend { message: String },
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
    use pcsc::{
        Card, Context, Error as PcscError, MAX_ATR_SIZE, Protocols, Scope, ShareMode, State,
    };
    use std::time::Duration;

    pub struct PcscReader {
        context: Context,
        card: Option<Card>,
        poll_interval: Duration,
    }

    impl PcscReader {
        pub fn new(poll_interval: Duration) -> Result<Self, ReaderError> {
            let context = Context::establish(Scope::User).map_err(ReaderError::from)?;
            Ok(Self {
                context,
                card: None,
                poll_interval,
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
            let name = readers_buf
                .first()
                .ok_or_else(|| ReaderError::backend("failed to obtain reader name from PC/SC"))?
                .as_str();
            let card = self
                .context
                .connect(name, ShareMode::Shared, Protocols::ANY)
                .map_err(ReaderError::from)?;
            self.card = Some(card);
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
                return Err(ReaderError::backend(format!(
                    "unexpected status word {:02X}{:02X}",
                    status[0], status[1]
                )));
            }
            Ok(CardUid::new(data.to_vec()))
        }

        fn poll(&mut self) -> Result<Option<ReaderEvent>, ReaderError> {
            if self.card.is_none() {
                self.connect_card()?;
            }

            let card = match self.card.as_mut() {
                Some(card) => card,
                None => return Ok(None),
            };

            let mut state = State::empty();
            let mut atr = [0u8; MAX_ATR_SIZE];
            let mut atr_len = atr.len();
            card.status(&mut state, None, &mut atr, &mut atr_len)
                .map_err(ReaderError::from)?;

            if !state.contains(State::PRESENT) {
                drop(card);
                self.card = None;
                return Ok(None);
            }

            match Self::read_uid(card) {
                Ok(uid) => Ok(Some(ReaderEvent::CardPresent { uid })),
                Err(err) => {
                    if matches!(
                        err,
                        ReaderError::Pcsc(PcscError::RemovedCard)
                            | ReaderError::Pcsc(PcscError::ResetCard)
                    ) {
                        drop(card);
                        self.card = None;
                        Ok(None)
                    } else {
                        Err(err)
                    }
                }
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
