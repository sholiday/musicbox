//! Debug web server exposing controller state, configuration, and manual
//! playback controls for development builds.

use crate::config::MusicBoxConfig;
use crate::controller::{
    AudioPlayer, CardUid, CardUidParseError, ControllerError, MusicBoxController, Track,
};
use crate::telemetry::{SharedStatus, StatusSnapshot};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};
use thiserror::Error;
use tracing::info;

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Musicbox Debug Dashboard</title>
  <script src="https://cdn.tailwindcss.com"></script>
</head>
<body class="bg-slate-950 text-slate-100 min-h-screen">
  <div class="max-w-6xl mx-auto px-6 py-10 space-y-8">
    <header class="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
      <div>
        <h1 class="text-3xl font-semibold">Musicbox Debug Dashboard</h1>
        <p class="text-sm text-slate-400">Monitor playback, edit card mappings, and trigger tracks manually.</p>
      </div>
      <div class="flex gap-3">
        <button id="pauseBtn" class="rounded-md bg-amber-500 px-4 py-2 font-medium text-slate-900 hover:bg-amber-400 focus:outline-none focus:ring-2 focus:ring-amber-300">Pause Playback</button>
        <button id="refreshBtn" class="rounded-md border border-slate-600 px-4 py-2 font-medium hover:bg-slate-800 focus:outline-none focus:ring-2 focus:ring-slate-500">Refresh Now</button>
      </div>
    </header>

    <div id="toast" class="hidden rounded-md border border-slate-700 bg-slate-900 px-4 py-3 text-sm"></div>

    <section class="grid gap-6 lg:grid-cols-3">
      <div class="rounded-xl border border-slate-800 bg-slate-900/70 p-6 backdrop-blur">
        <h2 class="mb-4 text-lg font-medium">Controller Status</h2>
        <dl class="space-y-3 text-sm">
          <div class="flex justify-between">
            <dt class="text-slate-400">Idle events</dt>
            <dd id="idleCount" class="font-mono">0</dd>
          </div>
          <div class="flex justify-between">
            <dt class="text-slate-400">Last update</dt>
            <dd id="lastUpdate" class="font-mono">–</dd>
          </div>
          <div>
            <dt class="text-slate-400">Last action</dt>
            <dd id="lastAction" class="mt-1 rounded-md bg-slate-800/60 p-2 text-xs">–</dd>
          </div>
          <div>
            <dt class="text-slate-400">Active card</dt>
            <dd id="activeCard" class="mt-1 font-mono">–</dd>
          </div>
          <div>
            <dt class="text-slate-400">Active track</dt>
            <dd id="activeTrack" class="mt-1 truncate text-sm">–</dd>
          </div>
        </dl>
      </div>

      <div class="lg:col-span-2 rounded-xl border border-slate-800 bg-slate-900/70 p-6 backdrop-blur">
        <div class="flex items-center justify-between">
          <h2 class="text-lg font-medium">Configured Cards</h2>
          <span id="libraryCount" class="text-xs text-slate-400">0 mappings</span>
        </div>
        <div class="mt-4 overflow-hidden rounded-lg border border-slate-800">
          <table class="min-w-full divide-y divide-slate-800">
            <thead class="bg-slate-900/70 text-left text-xs uppercase tracking-wide text-slate-400">
              <tr>
                <th class="px-4 py-3 font-medium">Card UID</th>
                <th class="px-4 py-3 font-medium">Track</th>
                <th class="px-4 py-3 font-medium text-right">Actions</th>
              </tr>
            </thead>
            <tbody id="libraryRows" class="divide-y divide-slate-800 text-sm"></tbody>
          </table>
        </div>
        <p id="libraryEmpty" class="mt-4 hidden rounded-md border border-slate-800 bg-slate-900/60 px-4 py-3 text-sm text-slate-300">
          No card mappings found. Update the config to add UIDs and tracks.
        </p>
      </div>
    </section>

    <section class="rounded-xl border border-slate-800 bg-slate-900/70 p-6 backdrop-blur">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-lg font-medium">Configuration</h2>
          <p id="configPath" class="text-xs text-slate-500 mt-1"></p>
        </div>
        <span id="configDirty" class="hidden rounded-full bg-amber-500/20 px-3 py-1 text-xs font-medium text-amber-300">Unsaved changes</span>
      </div>
      <textarea id="configEditor" class="mt-4 h-64 w-full rounded-lg border border-slate-700 bg-slate-950/70 p-3 font-mono text-sm focus:outline-none focus:ring-2 focus:ring-slate-500"></textarea>
      <div class="mt-4 flex flex-wrap gap-3">
        <button id="saveConfigBtn" class="rounded-md bg-indigo-500 px-4 py-2 text-sm font-medium text-slate-900 hover:bg-indigo-400 focus:outline-none focus:ring-2 focus:ring-indigo-300">Save Config</button>
        <button id="reloadConfigBtn" class="rounded-md border border-slate-600 px-4 py-2 text-sm font-medium hover:bg-slate-800 focus:outline-none focus:ring-2 focus:ring-slate-500">Reload from Disk</button>
      </div>
    </section>
  </div>

  <script>
    const toastEl = document.getElementById('toast');
    const idleCountEl = document.getElementById('idleCount');
    const lastUpdateEl = document.getElementById('lastUpdate');
    const lastActionEl = document.getElementById('lastAction');
    const activeCardEl = document.getElementById('activeCard');
    const activeTrackEl = document.getElementById('activeTrack');
    const libraryRowsEl = document.getElementById('libraryRows');
    const libraryEmptyEl = document.getElementById('libraryEmpty');
    const libraryCountEl = document.getElementById('libraryCount');
    const configEditorEl = document.getElementById('configEditor');
    const configPathEl = document.getElementById('configPath');
    const configDirtyEl = document.getElementById('configDirty');
    let configDirty = false;

    function showToast(message, isError = false) {
      toastEl.textContent = message;
      toastEl.classList.remove('hidden');
      toastEl.classList.toggle('border-red-500', isError);
      toastEl.classList.toggle('border-slate-700', !isError);
      toastEl.classList.toggle('text-red-300', isError);
      toastEl.classList.toggle('text-slate-200', !isError);
      setTimeout(() => toastEl.classList.add('hidden'), 4000);
    }

    function setConfigDirty(isDirty) {
      configDirty = isDirty;
      configDirtyEl.classList.toggle('hidden', !isDirty);
    }

    async function fetchJson(url, options = {}) {
      const response = await fetch(url, {
        headers: { 'Content-Type': 'application/json', ...(options.headers || {}) },
        ...options,
      });

      if (!response.ok) {
        let message = response.statusText;
        try {
          const body = await response.json();
          if (body && body.error) {
            message = body.error;
          }
        } catch (_) {
          message = await response.text();
        }
        throw new Error(message || 'Request failed');
      }

      if (response.status === 204) {
        return null;
      }
      return response.json();
    }

    function updateStatus(status) {
      idleCountEl.textContent = status.idle_events;
      lastUpdateEl.textContent = status.last_update || '–';
      lastActionEl.textContent = status.last_action || '–';
      activeCardEl.textContent = status.active_card || '–';
      activeTrackEl.textContent = status.active_track || '–';
    }

    function renderLibrary(entries, activeCard) {
      libraryRowsEl.innerHTML = '';
      if (!entries || entries.length === 0) {
        libraryEmptyEl.classList.remove('hidden');
        libraryCountEl.textContent = '0 mappings';
        return;
      }
      libraryEmptyEl.classList.add('hidden');
      libraryCountEl.textContent = entries.length + (entries.length === 1 ? ' mapping' : ' mappings');
      entries.forEach((entry) => {
        const row = document.createElement('tr');
        row.className = 'bg-slate-950/40 hover:bg-slate-900/60';
        if (entry.card === activeCard) {
          row.classList.add('bg-slate-800/60');
        }

        const cardCell = document.createElement('td');
        cardCell.className = 'px-4 py-3 font-mono text-xs';
        cardCell.textContent = entry.card;

        const trackCell = document.createElement('td');
        trackCell.className = 'px-4 py-3 text-sm';
        trackCell.textContent = entry.track;
        trackCell.title = entry.track;

        const actionsCell = document.createElement('td');
        actionsCell.className = 'px-4 py-3 text-right';
        const playBtn = document.createElement('button');
        playBtn.className = 'rounded-md bg-emerald-500 px-3 py-1 text-xs font-semibold text-slate-900 hover:bg-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-300';
        playBtn.textContent = 'Play';
        playBtn.addEventListener('click', () => triggerPlay(entry.card));

        actionsCell.appendChild(playBtn);
        row.appendChild(cardCell);
        row.appendChild(trackCell);
        row.appendChild(actionsCell);
        libraryRowsEl.appendChild(row);
      });
    }

    async function refreshStatusAndLibrary() {
      try {
        const status = await fetchJson('/api/status');
        updateStatus(status);
        const library = await fetchJson('/api/library');
        renderLibrary(library.entries, status.active_card);
      } catch (err) {
        showToast(err.message, true);
      }
    }

    async function loadConfig() {
      try {
        const config = await fetchJson('/api/config');
        configEditorEl.value = config.contents;
        configPathEl.textContent = config.path;
        setConfigDirty(false);
      } catch (err) {
        showToast(err.message, true);
      }
    }

    async function triggerPlay(card) {
      try {
        const result = await fetchJson('/api/play', {
          method: 'POST',
          body: JSON.stringify({ card_hex: card }),
        });
        updateStatus(result.status);
        const library = await fetchJson('/api/library');
        renderLibrary(library.entries, result.status.active_card);
        if (result.message) {
          showToast(result.message);
        }
      } catch (err) {
        showToast(err.message, true);
      }
    }

    async function pausePlayback() {
      try {
        const result = await fetchJson('/api/pause', { method: 'POST' });
        updateStatus(result.status);
        const library = await fetchJson('/api/library');
        renderLibrary(library.entries, result.status.active_card);
        if (result.message) {
          showToast(result.message);
        }
      } catch (err) {
        showToast(err.message, true);
      }
    }

    async function saveConfig() {
      try {
        const contents = configEditorEl.value;
        const result = await fetchJson('/api/config', {
          method: 'PUT',
          body: JSON.stringify({ contents }),
        });
        configEditorEl.value = result.contents;
        configPathEl.textContent = result.path;
        setConfigDirty(false);
        showToast('Configuration saved');
        await refreshStatusAndLibrary();
      } catch (err) {
        showToast(err.message, true);
      }
    }

    document.addEventListener('DOMContentLoaded', async () => {
      document.getElementById('pauseBtn').addEventListener('click', pausePlayback);
      document.getElementById('refreshBtn').addEventListener('click', refreshStatusAndLibrary);
      document.getElementById('saveConfigBtn').addEventListener('click', saveConfig);
      document.getElementById('reloadConfigBtn').addEventListener('click', loadConfig);
      configEditorEl.addEventListener('input', () => setConfigDirty(true));

      await loadConfig();
      await refreshStatusAndLibrary();
      setInterval(refreshStatusAndLibrary, 4000);
    });
  </script>
</body>
</html>
"#;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("failed to build tokio runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("server error: {0}")]
    Server(#[source] std::io::Error),
}

pub struct DebugState<P: AudioPlayer + Send + 'static> {
    pub status: SharedStatus,
    pub controller: Arc<Mutex<MusicBoxController<P>>>,
    pub config_path: PathBuf,
}

impl<P: AudioPlayer + Send + 'static> Clone for DebugState<P> {
    fn clone(&self) -> Self {
        Self {
            status: self.status.clone(),
            controller: Arc::clone(&self.controller),
            config_path: self.config_path.clone(),
        }
    }
}

/// Starts a Tokio runtime and runs the Axum server.
pub fn serve<P: AudioPlayer + Send + 'static>(
    state: DebugState<P>,
    addr: SocketAddr,
) -> Result<(), WebError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(WebError::Runtime)?;

    rt.block_on(async move {
        let app = build_router(state);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|source| WebError::Bind { addr, source })?;
        tracing::info!(?addr, "starting debug server");
        axum::serve(listener, app.into_make_service())
            .await
            .map_err(WebError::Server)
    })
}

/// Creates the Axum router and defines the routes.
fn build_router<P: AudioPlayer + Send + 'static>(state: DebugState<P>) -> Router {
    Router::new()
        .route("/", get(index::<P>))
        .route("/api/status", get(get_status::<P>))
        .route("/api/library", get(get_library::<P>))
        .route("/api/config", get(get_config::<P>).put(update_config::<P>))
        .route("/api/play", post(play_card::<P>))
        .route("/api/pause", post(pause::<P>))
        .with_state(state)
}

/// Serves the HTML for the debug dashboard.
async fn index<P: AudioPlayer + Send + 'static>(
    State(_): State<DebugState<P>>,
) -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Returns the current status of the music box controller.
async fn get_status<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
) -> Json<StatusPayload> {
    Json(build_status(&state))
}

/// Returns the current music library.
async fn get_library<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
) -> Json<LibraryResponse> {
    let mut entries = {
        let guard = state.controller.lock().expect("controller lock");
        guard.library_entries()
    };
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let entries = entries
        .into_iter()
        .map(|(card, track)| LibraryEntry {
            card: card.to_hex_lowercase(),
            track: track.path().display().to_string(),
        })
        .collect();

    Json(LibraryResponse { entries })
}

/// Returns the current configuration.
async fn get_config<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let path = state.config_path.clone();
    let contents = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
        .await
        .map_err(ApiError::Join)?
        .map_err(ApiError::Io)?;

    Ok(Json(ConfigResponse {
        path: state.config_path.display().to_string(),
        contents,
    }))
}

/// Updates the configuration.
async fn update_config<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
    Json(request): Json<UpdateConfigRequest>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let contents = request.contents;
    let parsed = MusicBoxConfig::from_reader(contents.as_bytes())
        .map_err(|err| ApiError::InvalidConfig(err.to_string()))?;
    let library = parsed.clone().into_library();

    let path = state.config_path.clone();
    let contents_clone = contents.clone();
    tokio::task::spawn_blocking(move || std::fs::write(path, contents_clone))
        .await
        .map_err(ApiError::Join)?
        .map_err(ApiError::Io)?;

    {
        let mut guard = state.controller.lock().expect("controller lock");
        guard.replace_library(library);
    }

    info!(
        path = %state.config_path.display(),
        "debug UI wrote configuration"
    );

    Ok(Json(ConfigResponse {
        path: state.config_path.display().to_string(),
        contents,
    }))
}

/// Starts playback of a track associated with a card.
async fn play_card<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
    Json(request): Json<PlayRequest>,
) -> Result<Json<CommandResponse>, ApiError> {
    let uid = CardUid::from_hex(request.card_hex.trim()).map_err(ApiError::CardUid)?;
    let action = {
        let mut guard = state.controller.lock().expect("controller lock");
        guard.handle_card(&uid)
    }?;

    state.status.record_action(action.clone());
    let message = format!("{action:?}");
    let status = build_status(&state);

    Ok(Json(CommandResponse {
        status,
        message: Some(message),
    }))
}

/// Pauses the currently playing track.
async fn pause<P: AudioPlayer + Send + 'static>(
    State(state): State<DebugState<P>>,
) -> Result<Json<CommandResponse>, ApiError> {
    let maybe_action = {
        let mut guard = state.controller.lock().expect("controller lock");
        guard.pause_playback()
    }?;

    let status = build_status(&state);
    let message = match maybe_action {
        Some(action) => {
            state.status.record_action(action.clone());
            format!("{action:?}")
        }
        None => "No active playback to pause".to_string(),
    };

    Ok(Json(CommandResponse {
        status,
        message: Some(message),
    }))
}

fn build_status<P: AudioPlayer + Send + 'static>(state: &DebugState<P>) -> StatusPayload {
    let snapshot = state.status.snapshot();
    let active = {
        let guard = state.controller.lock().expect("controller lock");
        guard.active()
    };
    StatusPayload::from_snapshot(snapshot, active)
}

#[derive(Debug, Serialize)]
struct StatusPayload {
    idle_events: u64,
    last_action: Option<String>,
    last_update: Option<String>,
    active_card: Option<String>,
    active_track: Option<String>,
}

impl StatusPayload {
    fn from_snapshot(snapshot: StatusSnapshot, active: Option<(CardUid, Track)>) -> StatusPayload {
        let last_action = snapshot.last_action.map(|action| format!("{action:?}"));
        let last_update = snapshot
            .last_update
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs().to_string());

        let (active_card, active_track) = active
            .map(|(card, track)| {
                (
                    Some(card.to_hex_lowercase()),
                    Some(track.path().display().to_string()),
                )
            })
            .unwrap_or((None, None));

        StatusPayload {
            idle_events: snapshot.idle_events,
            last_action,
            last_update,
            active_card,
            active_track,
        }
    }
}

#[derive(Debug, Serialize)]
struct LibraryResponse {
    entries: Vec<LibraryEntry>,
}

#[derive(Debug, Serialize)]
struct LibraryEntry {
    card: String,
    track: String,
}

#[derive(Debug, Serialize)]
struct ConfigResponse {
    path: String,
    contents: String,
}

#[derive(Debug, Deserialize)]
struct UpdateConfigRequest {
    contents: String,
}

#[derive(Debug, Deserialize)]
struct PlayRequest {
    card_hex: String,
}

#[derive(Debug, Serialize)]
struct CommandResponse {
    status: StatusPayload,
    message: Option<String>,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("card uid parse error: {0}")]
    CardUid(#[from] CardUidParseError),
    #[error("controller error: {0}")]
    Controller(#[from] ControllerError),
    #[error("config validation failed: {0}")]
    InvalidConfig(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::CardUid(_) | ApiError::InvalidConfig(_) => StatusCode::BAD_REQUEST,
            ApiError::Controller(ControllerError::TrackNotFound) => StatusCode::NOT_FOUND,
            ApiError::Controller(_) => StatusCode::BAD_REQUEST,
            ApiError::Io(_) | ApiError::Join(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = Json(ErrorResponse {
            error: self.to_string(),
        });
        (status, body).into_response()
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::ControllerAction;

    #[test]
    fn status_payload_formats_fields() {
        let snapshot = StatusSnapshot {
            idle_events: 5,
            last_action: Some(ControllerAction::Started {
                card: CardUid::new(vec![0xde, 0xad]),
                track: Track::new("track.mp3".into()),
            }),
            last_update: Some(UNIX_EPOCH + std::time::Duration::from_secs(42)),
        };

        let payload = StatusPayload::from_snapshot(
            snapshot,
            Some((
                CardUid::new(vec![0xca, 0xfe]),
                Track::new("other.mp3".into()),
            )),
        );

        assert_eq!(payload.idle_events, 5);
        assert!(payload.last_action.as_ref().unwrap().contains("Started"));
        assert_eq!(payload.last_update.as_deref(), Some("42"));
        assert_eq!(payload.active_card.as_deref(), Some("cafe"));
        assert_eq!(payload.active_track.as_deref(), Some("other.mp3"));
    }
}
