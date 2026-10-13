use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};

use crate::wire::{self, CLOSE, FRAME, HANDSHAKE, PING, PONG};

/// What the app wants shown. `None` on the feed clears the presence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub details: Option<String>,
    pub state: Option<String>,
    /// Unix seconds; Discord renders the elapsed time since.
    pub started_at: Option<i64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
}

impl Activity {
    fn to_wire(&self, show_elapsed: bool, default_image: Option<&str>) -> serde_json::Value {
        let mut activity = serde_json::Map::new();
        if let Some(v) = &self.details {
            activity.insert("details".into(), v.clone().into());
        }
        if let Some(v) = &self.state {
            activity.insert("state".into(), v.clone().into());
        }
        if let (true, Some(start)) = (show_elapsed, self.started_at) {
            activity.insert("timestamps".into(), json!({ "start": start }));
        }
        let image = self.large_image.as_deref().or(default_image);
        if let Some(image) = image {
            let mut assets = serde_json::Map::new();
            assets.insert("large_image".into(), image.into());
            if let Some(text) = &self.large_text {
                assets.insert("large_text".into(), text.clone().into());
            }
            activity.insert("assets".into(), assets.into());
        }
        activity.into()
    }
}

pub struct Settings {
    pub client_id: String,
    pub show_elapsed: bool,
    pub default_image: Option<String>,
}

#[derive(Debug)]
pub enum Ended {
    /// Discord closed the connection or sent a close frame.
    Closed,
    Rejected(String),
}

/// One connection's lifetime: handshake, then mirror `feed` into Discord until it drops.
/// `on_ready` runs once Discord has accepted the handshake.
pub async fn run<S>(
    stream: S,
    settings: &Settings,
    mut feed: watch::Receiver<Option<Activity>>,
    on_ready: impl FnOnce(),
) -> io::Result<Ended>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut read, mut write) = tokio::io::split(stream);
    wire::write_frame(&mut write, HANDSHAKE, &json!({ "v": 1, "client_id": settings.client_id })).await?;
    let (opcode, reply) = wire::read_frame(&mut read).await?;
    if opcode == CLOSE {
        return Ok(Ended::Rejected(reply["message"].as_str().unwrap_or("closed during handshake").to_owned()));
    }
    if reply["evt"] != "READY" {
        return Ok(Ended::Rejected(format!("unexpected handshake reply: {reply}")));
    }
    on_ready();

    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<(u32, serde_json::Value)>();
    let reader = async {
        loop {
            let (opcode, payload) = wire::read_frame(&mut read).await?;
            match opcode {
                PING => {
                    let _ = control_tx.send((PONG, payload));
                }
                CLOSE => return Ok(Ended::Closed),
                _ => {}
            }
        }
    };
    let writer = async {
        let pid = std::process::id();
        let mut nonce = 0u64;
        // The first value is whatever the app set before the connection existed.
        let mut pending = true;
        loop {
            if pending {
                pending = false;
                nonce += 1;
                let activity = feed
                    .borrow_and_update()
                    .as_ref()
                    .map(|a| a.to_wire(settings.show_elapsed, settings.default_image.as_deref()));
                let frame = json!({
                    "cmd": "SET_ACTIVITY",
                    "args": { "pid": pid, "activity": activity },
                    "nonce": nonce.to_string(),
                });
                wire::write_frame(&mut write, FRAME, &frame).await?;
            }
            tokio::select! {
                changed = feed.changed() => {
                    if changed.is_err() {
                        return Ok::<Ended, io::Error>(Ended::Closed);
                    }
                    pending = true;
                }
                Some((opcode, payload)) = control_rx.recv() => {
                    wire::write_frame(&mut write, opcode, &payload).await?;
                }
            }
        }
    };
    tokio::select! {
        ended = reader => ended,
        ended = writer => ended,
    }
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
