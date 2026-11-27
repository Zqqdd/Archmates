//! The presence protocol (`archmates/presence/1`): saying hello to friends, exchanging
//! profiles and pictures, and delivering chat messages (queued ones too).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId};

use super::state::Shared;
use crate::common::chat;
use crate::common::profile::{AVATAR_MAX, Profile, read_avatar};
use crate::common::proto::{self, ALPN, CODE_NOT_FRIEND, JSON_MAX, Request, Response};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

// ── Server side: answer friends who contact us ──────────────────────────────

#[derive(Clone)]
pub(super) struct Greeter(pub(super) Arc<Shared>);

impl std::fmt::Debug for Greeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Greeter")
    }
}

impl ProtocolHandler for Greeter {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id();
        let shared = &self.0;
        // Refuse: invisible mode (shown "offline"), or not in our list ("hasn't added you").
        let invisible = shared.paths.is_invisible();
        if invisible || !shared.is_friend(&id) {
            proto::refuse(&connection, invisible);
            return Ok(());
        }
        if let Err(err) = serve(shared, &connection, id).await {
            eprintln!("[incoming {}] {err:#}", short(&id));
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

async fn serve(shared: &Arc<Shared>, connection: &Connection, id: EndpointId) -> Result<()> {
    // One stream per request, until the friend closes the connection.
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let bytes = recv.read_to_end(JSON_MAX).await?;
        let request: Request = serde_json::from_slice(&bytes).context("unreadable request")?;
        match request {
            Request::Hello { profile } => {
                shared.seen(id, profile);
                let response = Response { profile: Profile::local() };
                send.write_all(&serde_json::to_vec(&response)?).await?;
                let _ = shared.publish();
            }
            Request::Avatar => {
                let image = read_avatar(&Profile::local().user).unwrap_or_default();
                send.write_all(&image).await?;
            }
            Request::Message { id: msg_id, text, ms } => {
                let text: String = text.chars().take(chat::MESSAGE_MAX).collect();
                let code = id.to_string();
                let message = chat::Message {
                    id: msg_id,
                    dir: "in".into(),
                    text: text.clone(),
                    ms,
                    delivered: true,
                    file: None,
                    voice: None,
                    received_ms: None,
                };
                if chat::receive(&shared.paths, &code, message)? {
                    let nickname =
                        shared.friends.lock().unwrap().get(&id).map(|f| f.nickname.clone()).unwrap_or_default();
                    notify_message(&code, &nickname, &text);
                }
                send.write_all(b"ok").await?;
                let _ = shared.publish();
            }
        }
        send.finish()?;
    }
    Ok(())
}

// ── Client side: contact a friend ───────────────────────────────────────────

pub(super) async fn greet(endpoint: &Endpoint, shared: &Arc<Shared>, id: EndpointId) -> Result<()> {
    let connection = tokio::time::timeout(CONNECT_TIMEOUT, endpoint.connect(id, ALPN))
        .await
        .map_err(|_| anyhow!("unreachable (timed out)"))??;

    let result = exchange(shared, &connection, id).await;
    if result.is_err() && closed_as_not_friend(&connection) {
        shared.set_pending(id, true);
        connection.close(0u32.into(), b"ok");
        return Err(anyhow!("hasn't added you yet"));
    }
    connection.close(0u32.into(), b"ok");
    result
}

async fn exchange(shared: &Arc<Shared>, connection: &Connection, id: EndpointId) -> Result<()> {
    let (mut send, mut recv) = connection.open_bi().await?;
    let request = Request::Hello { profile: Profile::local() };
    send.write_all(&serde_json::to_vec(&request)?).await?;
    send.finish()?;
    let bytes = recv.read_to_end(JSON_MAX).await?;
    let response: Response = serde_json::from_slice(&bytes).context("unreadable response")?;

    let hash = response.profile.avatar_hash.clone();
    shared.seen(id, response.profile);

    if hash.is_some() && !shared.avatar_up_to_date(&id, &hash) {
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(&serde_json::to_vec(&Request::Avatar)?).await?;
        send.finish()?;
        let image = recv.read_to_end(AVATAR_MAX).await?;
        if !image.is_empty() {
            shared.save_avatar(id, &image)?;
        }
    }

    // Deliver messages written while the friend was offline.
    let code = id.to_string();
    for m in chat::pending(&shared.paths, &code) {
        let (mut send, mut recv) = connection.open_bi().await?;
        let request = Request::Message { id: m.id.clone(), text: m.text.clone(), ms: m.ms };
        send.write_all(&serde_json::to_vec(&request)?).await?;
        send.finish()?;
        if recv.read_to_end(16).await? == b"ok" {
            chat::mark_delivered(&shared.paths, &code, &m.id)?;
        } else {
            break;
        }
    }
    Ok(())
}

fn notify_message(code: &str, nickname: &str, text: &str) {
    let preview: String = text.chars().take(120).collect();
    crate::platform::notify(nickname, &preview, None, true, crate::platform::OnClick::OpenChat(code.to_string()));
}

fn closed_as_not_friend(connection: &Connection) -> bool {
    // The other side closes with code 403 when we are not in its list (503 = invisible).
    proto::close_code(connection) == Some(CODE_NOT_FRIEND as u64)
}

pub(super) fn short(id: &EndpointId) -> String {
    id.to_string().chars().take(8).collect()
}
