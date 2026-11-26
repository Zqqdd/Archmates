//! The presence service (`archmates daemon`).
//!
//! - listens for incoming connections from your friends and answers with your profile;
//! - every 30 s, contacts each of your friends;
//! - writes everyone's status to ~/.cache/archmates/state.json for the widget;
//! - sends a notification when a friend comes online.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};

use crate::config::{Friend, FriendList, Paths, write_atomic};
use crate::profile::{AVATAR_MAX, Profile, fingerprint, read_avatar};
use crate::chat;
use crate::proto::{ALPN, CODE_NOT_FRIEND, JSON_MAX, Request, Response};
use crate::transfer::{FILE_ALPN, FileReceiver};
use crate::update;

const INTERVAL: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// A friend is "online" if we exchanged with them less than this long ago.
const ONLINE_IF_SEEN_WITHIN_MS: u64 = 75_000;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ── Shared state, as written to state.json ──────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Online,
    Offline,
    /// The friend has not added you on their side yet.
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendState {
    pub nickname: String,
    pub code: String,
    pub status: Status,
    /// Last successful exchange (ms since 1970), kept across restarts.
    pub last_seen_ms: Option<u64>,
    pub profile: Option<Profile>,
    /// Local path of their cached profile picture.
    pub avatar: Option<String>,
    #[serde(default)]
    pub pending: bool,
    /// Messages received since you last opened the conversation.
    #[serde(default)]
    pub unread: usize,
    /// Files this friend wants to send you, waiting for your answer.
    #[serde(default)]
    pub offers: Vec<Offer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub name: String,
    pub size: u64,
}

struct PendingOffer {
    friend: EndpointId,
    offer: Offer,
    answer: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Me {
    pub code: String,
    pub invisible: bool,
    pub updated_ms: u64,
    pub version: String,
    /// Newer version published on GitHub, if any.
    #[serde(default)]
    pub update_available: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
    pub me: Me,
    pub friends: Vec<FriendState>,
}

struct Shared {
    paths: Paths,
    me: EndpointId,
    friends: Mutex<BTreeMap<EndpointId, FriendState>>,
    notified: Mutex<HashSet<EndpointId>>,
    update_available: Mutex<Option<String>>,
    offers: Mutex<HashMap<String, PendingOffer>>,
}

impl Shared {
    /// Syncs the state with friends.toml (picked up without restarting the service).
    fn reload_list(&self) -> Result<BTreeMap<EndpointId, Friend>> {
        let list = FriendList::load(&self.paths)?.by_id();
        let mut friends = self.friends.lock().unwrap();
        friends.retain(|id, _| list.contains_key(id));
        for (id, f) in &list {
            let entry = friends.entry(*id).or_insert_with(|| FriendState {
                nickname: f.nickname.clone(),
                code: f.code.clone(),
                status: Status::Offline,
                last_seen_ms: None,
                profile: None,
                avatar: None,
                pending: false,
                unread: 0,
                offers: Vec::new(),
            });
            entry.nickname = f.nickname.clone();
        }
        Ok(list)
    }

    /// Answers a file offer (from the widgets, the CLI or the notification).
    fn answer_offer(&self, offer_id: &str, accept: bool) -> bool {
        let found = self.offers.lock().unwrap().remove(offer_id);
        let ok = found.map(|o| o.answer.send(accept).is_ok()).unwrap_or(false);
        let _ = self.publish();
        ok
    }

    fn is_friend(&self, id: &EndpointId) -> bool {
        FriendList::load(&self.paths).map(|l| l.by_id().contains_key(id)).unwrap_or(false)
    }

    /// Records a successful exchange with a friend.
    fn seen(&self, id: EndpointId, profile: Profile) {
        let mut friends = self.friends.lock().unwrap();
        if let Some(f) = friends.get_mut(&id) {
            f.last_seen_ms = Some(now_ms());
            f.profile = Some(profile);
            f.pending = false;
        }
    }

    fn set_pending(&self, id: EndpointId, pending: bool) {
        if let Some(f) = self.friends.lock().unwrap().get_mut(&id) {
            f.pending = pending;
        }
    }

    fn avatar_up_to_date(&self, id: &EndpointId, remote_hash: &Option<String>) -> bool {
        let friends = self.friends.lock().unwrap();
        let Some(f) = friends.get(id) else { return true };
        let Some(hash) = remote_hash else { return true };
        let file = self.paths.avatars().join(format!("{id}-{hash}"));
        f.avatar.as_deref() == Some(file.to_string_lossy().as_ref()) && file.exists()
    }

    fn save_avatar(&self, id: EndpointId, bytes: &[u8]) -> Result<()> {
        let hash = fingerprint(bytes);
        let dir = self.paths.avatars();
        std::fs::create_dir_all(&dir)?;
        // Remove this friend's older pictures.
        for entry in std::fs::read_dir(&dir)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&format!("{id}-")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let file = dir.join(format!("{id}-{hash}"));
        write_atomic(&file, bytes)?;
        if let Some(f) = self.friends.lock().unwrap().get_mut(&id) {
            f.avatar = Some(file.to_string_lossy().into_owned());
        }
        Ok(())
    }

    /// Recomputes statuses, writes state.json and sends notifications.
    fn publish(&self) -> Result<()> {
        let invisible = self.paths.is_invisible();
        let t = now_ms();
        let mut came_online = Vec::new();
        let list: Vec<FriendState> = {
            let mut friends = self.friends.lock().unwrap();
            let mut notified = self.notified.lock().unwrap();
            let mut offers = self.offers.lock().unwrap();
            offers.retain(|_, o| !o.answer.is_closed()); // expired (no answer in time)
            for (id, f) in friends.iter_mut() {
                f.unread = chat::unread(&self.paths, &f.code);
                f.offers = offers.values().filter(|o| o.friend == *id).map(|o| o.offer.clone()).collect();
                let recent = f.last_seen_ms.is_some_and(|v| t.saturating_sub(v) < ONLINE_IF_SEEN_WITHIN_MS);
                f.status = if invisible {
                    Status::Offline
                } else if recent {
                    Status::Online
                } else if f.pending {
                    Status::Pending
                } else {
                    Status::Offline
                };
                if f.status == Status::Online {
                    if notified.insert(*id) {
                        came_online.push(f.clone());
                    }
                } else {
                    notified.remove(id);
                }
            }
            friends.values().cloned().collect()
        };
        let file = StateFile {
            me: Me {
                code: self.me.to_string(),
                invisible,
                updated_ms: t,
                version: update::CURRENT.to_string(),
                update_available: self.update_available.lock().unwrap().clone(),
            },
            friends: list,
        };
        write_atomic(&self.paths.state(), serde_json::to_string_pretty(&file)?.as_bytes())?;
        for f in came_online {
            notify(&f);
        }
        Ok(())
    }
}

fn notify(f: &FriendState) {
    let body = match &f.profile {
        Some(p) => format!("{}@{} · {}", p.user, p.host, p.distro),
        None => String::new(),
    };
    let mut cmd = std::process::Command::new("notify-send");
    cmd.args(["-a", "Archmates", "-u", "low"]);
    if let Some(a) = &f.avatar {
        cmd.args(["-i", a]);
    }
    cmd.arg(format!("{} is online", f.nickname)).arg(body);
    let _ = cmd.spawn();
}

// ── Server side: answer friends who contact us ──────────────────────────────

#[derive(Clone)]
struct Greeter(Arc<Shared>);

impl std::fmt::Debug for Greeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Greeter")
    }
}

impl ProtocolHandler for Greeter {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id();
        let shared = &self.0;
        // Refuse: not in our list, or invisible mode.
        if shared.paths.is_invisible() || !shared.is_friend(&id) {
            connection.close(CODE_NOT_FRIEND.into(), b"not a friend");
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
                let message = chat::Message { id: msg_id, dir: "in".into(), text: text.clone(), ms, delivered: true, file: None };
                if chat::receive(&shared.paths, &code, message)? {
                    let nickname = shared.friends.lock().unwrap().get(&id).map(|f| f.nickname.clone()).unwrap_or_default();
                    notify_message(&nickname, &text);
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

async fn greet(endpoint: &Endpoint, shared: &Arc<Shared>, id: EndpointId) -> Result<()> {
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

fn notify_message(nickname: &str, text: &str) {
    let preview: String = text.chars().take(120).collect();
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Archmates", "-u", "normal"])
        .arg(format!("{nickname}"))
        .arg(preview)
        .spawn();
}

/// Friend id and code from a nickname or a code.
fn resolve(shared: &Shared, who: &str) -> Option<(EndpointId, String, String)> {
    FriendList::load(&shared.paths).ok()?.by_id().into_iter().find_map(|(id, f)| {
        (f.nickname == who || f.code == who).then(|| (id, f.code.clone(), f.nickname.clone()))
    })
}

/// Requests from the `archmates` command and the widgets (see ipc.rs).
async fn handle_command(endpoint: Endpoint, shared: Arc<Shared>, command: crate::ipc::Command) -> crate::ipc::Answer {
    use crate::ipc::{Answer, Command};
    let fail = |message: String| Answer { ok: false, message };
    match command {
        Command::SendMessage { to, text } => {
            let Some((id, code, nickname)) = resolve(&shared, &to) else {
                return fail(format!("no friend named \"{to}\""));
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return fail("empty message".into());
            }
            if text.chars().count() > chat::MESSAGE_MAX {
                return fail(format!("messages are limited to {} characters", chat::MESSAGE_MAX));
            }
            let message = chat::Message { id: chat::new_id(), dir: "out".into(), text, ms: now_ms(), delivered: false, file: None };
            if let Err(err) = chat::append(&shared.paths, &code, &message) {
                return fail(format!("{err:#}"));
            }
            let _ = greet(&endpoint, &shared, id).await;
            let _ = shared.publish();
            if chat::pending(&shared.paths, &code).is_empty() {
                Answer { ok: true, message: format!("sent to {nickname}") }
            } else {
                Answer { ok: true, message: format!("{nickname} is offline: the message will be sent when they come back") }
            }
        }
        Command::AnswerFile { id, accept } => {
            if shared.answer_offer(&id, accept) {
                Answer { ok: true, message: if accept { "file accepted".into() } else { "file declined".into() } }
            } else {
                fail("this file offer has expired".into())
            }
        }
        Command::SendFile { to, path } => {
            let Some((id, code, nickname)) = resolve(&shared, &to) else {
                return fail(format!("no friend named \"{to}\""));
            };
            match crate::transfer::send(&endpoint, id, std::path::Path::new(&path)).await {
                Ok(name) => {
                    let message = chat::Message {
                        id: chat::new_id(), dir: "out".into(), text: format!("📎 {name}"),
                        ms: now_ms(), delivered: true, file: Some(path.clone()),
                    };
                    let _ = chat::append(&shared.paths, &code, &message);
                    let _ = shared.publish();
                    Answer { ok: true, message: format!("{name} sent to {nickname}") }
                }
                Err(err) => fail(format!("{err:#}")),
            }
        }
    }
}

fn closed_as_not_friend(connection: &Connection) -> bool {
    // The other side closes with code 403 when we are not in its list.
    connection
        .close_reason()
        .map(|r| format!("{r:?}").contains(&format!("error_code: {CODE_NOT_FRIEND}")))
        .unwrap_or(false)
}

fn short(id: &EndpointId) -> String {
    id.to_string().chars().take(8).collect()
}

// ── Update check ────────────────────────────────────────────────────────────

const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// Checks GitHub one minute after start, then once a day. Notifies once per new version.
async fn check_updates(shared: Arc<Shared>) {
    if std::env::var_os("ARCHMATES_NO_UPDATE_CHECK").is_some() {
        return;
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        match update::latest_version().await {
            Ok(latest) if update::is_newer(&latest, update::CURRENT) => {
                *shared.update_available.lock().unwrap() = Some(latest.clone());
                let marker = shared.paths.cache.join("update-notified");
                let already = std::fs::read_to_string(&marker).ok().is_some_and(|v| v.trim() == latest);
                if !already {
                    let _ = std::process::Command::new("notify-send")
                        .args(["-a", "Archmates", "-u", "normal"])
                        .arg(format!("Archmates {latest} is available"))
                        .arg(format!("You have {}. Update with: {}", update::CURRENT, update::how_to_update()))
                        .spawn();
                    let _ = write_atomic(&marker, latest.as_bytes());
                }
                let _ = shared.publish();
            }
            Ok(_) => {
                *shared.update_available.lock().unwrap() = None;
            }
            Err(err) => eprintln!("update check: {err:#}"),
        }
        tokio::time::sleep(UPDATE_CHECK_EVERY).await;
    }
}

// ── Main loop ───────────────────────────────────────────────────────────────

pub async fn run() -> Result<()> {
    let paths = Paths::detect();
    let key = crate::config::load_or_create_key(&paths)?;
    let me = key.public();

    // Restore the last known state (pictures, "last seen") after a restart.
    let mut initial = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(paths.state()) {
        if let Ok(old) = serde_json::from_str::<StateFile>(&txt) {
            for f in old.friends {
                if let Ok(id) = f.code.parse::<EndpointId>() {
                    initial.insert(id, FriendState { pending: false, ..f });
                }
            }
        }
    }
    let shared = Arc::new(Shared {
        paths,
        me,
        friends: Mutex::new(initial),
        notified: Mutex::new(HashSet::new()),
        offers: Mutex::new(HashMap::new()),
        update_available: Mutex::new(None),
    });
    tokio::spawn(check_updates(shared.clone()));
    tokio::spawn(crate::widget::supervise(Paths::detect()));

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(key)
        .bind()
        .await
        .context("could not start the P2P network")?;
    let on_offer = {
        let shared = shared.clone();
        Arc::new(move |id: EndpointId, name: String, size: u64| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let offer = Offer { id: chat::new_id(), name: name.clone(), size };
            let nickname = shared.friends.lock().unwrap().get(&id).map(|f| f.nickname.clone()).unwrap_or_default();
            shared.offers.lock().unwrap().insert(offer.id.clone(), PendingOffer { friend: id, offer: offer.clone(), answer: tx });
            let _ = shared.publish();
            // Notification with Accept / Decline buttons (the widgets show the same buttons).
            let (shared, offer_id) = (shared.clone(), offer.id.clone());
            std::thread::spawn(move || {
                let out = std::process::Command::new("notify-send")
                    .args(["-a", "Archmates", "-u", "critical", "--wait", "--action=accept=Accept", "--action=decline=Decline"])
                    .arg(format!("{nickname} wants to send you a file"))
                    .arg(format!("{name} ({:.1} MB)", size as f64 / 1_048_576.0))
                    .output();
                if let Ok(out) = out {
                    match String::from_utf8_lossy(&out.stdout).trim() {
                        "accept" => { shared.answer_offer(&offer_id, true); }
                        "decline" => { shared.answer_offer(&offer_id, false); }
                        _ => {}
                    }
                }
            });
            rx
        }) as Arc<crate::transfer::OfferFn>
    };
    let on_saved = {
        let shared = shared.clone();
        Arc::new(move |id: EndpointId, name: String, saved: std::path::PathBuf| {
            let code = id.to_string();
            let nickname = shared.friends.lock().unwrap().get(&id).map(|f| f.nickname.clone()).unwrap_or_default();
            let message = chat::Message {
                id: chat::new_id(), dir: "in".into(), text: format!("📎 {name}"),
                ms: now_ms(), delivered: true, file: Some(saved.to_string_lossy().into_owned()),
            };
            let _ = chat::append(&shared.paths, &code, &message);
            let _ = std::process::Command::new("notify-send")
                .args(["-a", "Archmates", "-u", "normal"])
                .arg(format!("File received from {nickname}"))
                .arg(format!("{name}\nSaved in {}", saved.parent().map(|p| p.display().to_string()).unwrap_or_default()))
                .spawn();
            let _ = shared.publish();
        }) as Arc<crate::transfer::SavedFn>
    };
    let on_file = FileReceiver { on_offer, on_saved };
    let router = Router::builder(endpoint.clone())
        .accept(ALPN, Greeter(shared.clone()))
        .accept(FILE_ALPN, on_file)
        .spawn();
    {
        let (endpoint, shared) = (endpoint.clone(), shared.clone());
        tokio::spawn(async move {
            let result = crate::ipc::serve(move |cmd| handle_command(endpoint.clone(), shared.clone(), cmd)).await;
            if let Err(err) = result {
                eprintln!("control socket: {err:#}");
            }
        });
    }
    eprintln!("archmates: service started, your code is {me}");
    let _ = tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await;
    eprintln!("archmates: connected to the network");

    let mut ticker = tokio::time::interval(INTERVAL);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        // After an update the program file is replaced: exit so systemd restarts the new version.
        if update::binary_replaced() {
            eprintln!("archmates: new version installed, restarting");
            router.shutdown().await.ok();
            std::process::exit(75);
        }
        let list = match shared.reload_list() {
            Ok(l) => l,
            Err(err) => {
                eprintln!("friends.toml: {err:#}");
                continue;
            }
        };
        if !shared.paths.is_invisible() {
            let mut tasks = Vec::new();
            for (id, friend) in list {
                let (ep, sh) = (endpoint.clone(), shared.clone());
                tasks.push(tokio::spawn(async move {
                    match greet(&ep, &sh, id).await {
                        Ok(()) => sh.set_pending(id, false),
                        Err(err) => eprintln!("[{}] {err:#}", friend.nickname),
                    }
                }));
            }
            for t in tasks {
                let _ = t.await;
            }
        }
        if let Err(err) = shared.publish() {
            eprintln!("state.json: {err:#}");
        }
    }

    router.shutdown().await.ok();
    Ok(())
}
