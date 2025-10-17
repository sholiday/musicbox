use crate::telemetry::{SharedStatus, StatusSnapshot};
use axum::{Json, Router, routing::get};
use serde::Serialize;
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("failed to build tokio runtime: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("server error: {0}")]
    Server(#[from] axum::Error),
}

pub fn serve(status: SharedStatus, addr: SocketAddr) -> Result<(), WebError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(WebError::Runtime)?;

    rt.block_on(async move {
        let app = build_router(status);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|source| WebError::Bind { addr, source })?;
        tracing::info!(?addr, "starting debug server");
        axum::serve(listener, app.into_make_service())
            .await
            .map_err(WebError::Server)
    })
}

fn build_router(status: SharedStatus) -> Router {
    let status_clone = status.clone();
    Router::new().route(
        "/status",
        get(move || {
            let status = status_clone.clone();
            async move { Json(StatusResponse::from(status.snapshot())) }
        }),
    )
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    idle_events: u64,
    last_action: Option<String>,
    last_update: Option<String>,
}

impl From<StatusSnapshot> for StatusResponse {
    fn from(snapshot: StatusSnapshot) -> Self {
        let last_action = snapshot.last_action.map(|action| format!("{action:?}"));
        let last_update = snapshot
            .last_update
            .and_then(|ts| ts.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs().to_string());

        Self {
            idle_events: snapshot.idle_events,
            last_action,
            last_update,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_serializes_action() {
        let snapshot = StatusSnapshot {
            idle_events: 2,
            last_action: Some(crate::controller::ControllerAction::Started {
                card: crate::controller::CardUid::new(vec![1, 2, 3]),
                track: crate::controller::Track::new("song.mp3".into()),
            }),
            last_update: Some(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5),
            ),
        };

        let response = StatusResponse::from(snapshot);
        assert_eq!(response.idle_events, 2);
        assert!(response.last_action.unwrap().contains("Started"));
        assert_eq!(response.last_update.unwrap(), "5");
    }
}
