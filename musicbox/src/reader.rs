use crate::controller::CardUid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ReaderError {
    #[error("reader backend error: {message}")]
    Backend { message: String },
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
