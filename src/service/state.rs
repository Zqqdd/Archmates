//! What the service knows about your friends, and `state.json`: the file every widget
//! (Noctalia, universal, Winmates) reads to show who is online.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use anyhow::Result;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};

use crate::common::config::{Friend, FriendList, Paths, write_atomic};
use crate::common::profile::{Profile, fingerprint};
use crate::common::time::now_ms;
use crate::common::{chat, update};

/// A friend is "online" if we exchanged with them less than this long ago.
const ONLINE_IF_SEEN_WITHIN_MS: u64 = 75_000;

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

pub(super) struct PendingOffer {
    pub(super) friend: EndpointId,
    pub(super) offer: Offer,
    pub(super) answer: tokio::sync::oneshot::Sender<bool>,
    /// The Accept / Decline notification, closed when the offer is answered elsewhere.
    pub(super) notification: Option<u32>,
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

pub(super) struct Shared {
    pub(super) paths: Paths,
    pub(super) me: EndpointId,
    pub(super) friends: Mutex<BTreeMap<EndpointId, FriendState>>,
    pub(super) notified: Mutex<HashSet<EndpointId>>,
    pub(super) update_available: Mutex<Option<String>>,
    pub(super) offers: Mutex<HashMap<String, PendingOffer>>,
    pub(super) voice: super::voice_desk::Desk,
}

impl Shared {
    /// Syncs the state with friends.toml (picked up without restarting the service).
    pub(super) fn reload_list(&self) -> Result<BTreeMap<EndpointId, Friend>> {
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
    pub(super) fn answer_offer(&self, offer_id: &str, accept: bool) -> bool {
        let found = self.offers.lock().unwrap().remove(offer_id);
        let ok = match found {
            Some(o) => {
                if let Some(n) = o.notification {
                    crate::platform::close_notification(n);
                }
                o.answer.send(accept).is_ok()
            }
            None => false,
        };
        let _ = self.publish();
        ok
    }

    /// Remembers the notification shown for an offer.
    pub(super) fn set_offer_notification(&self, offer_id: &str, notification: u32) {
        if let Some(o) = self.offers.lock().unwrap().get_mut(offer_id) {
            o.notification = Some(notification);
        }
    }

    pub(super) fn is_friend(&self, id: &EndpointId) -> bool {
        FriendList::load(&self.paths).map(|l| l.by_id().contains_key(id)).unwrap_or(false)
    }

    /// Records a successful exchange with a friend.
    pub(super) fn seen(&self, id: EndpointId, profile: Profile) {
        let mut friends = self.friends.lock().unwrap();
        if let Some(f) = friends.get_mut(&id) {
            f.last_seen_ms = Some(now_ms());
            f.profile = Some(profile);
            f.pending = false;
        }
    }

    pub(super) fn set_pending(&self, id: EndpointId, pending: bool) {
        if let Some(f) = self.friends.lock().unwrap().get_mut(&id) {
            f.pending = pending;
        }
    }

    pub(super) fn avatar_up_to_date(&self, id: &EndpointId, remote_hash: &Option<String>) -> bool {
        let friends = self.friends.lock().unwrap();
        let Some(f) = friends.get(id) else { return true };
        let Some(hash) = remote_hash else { return true };
        let file = self.paths.avatars().join(format!("{id}-{hash}"));
        f.avatar.as_deref() == Some(file.to_string_lossy().as_ref()) && file.exists()
    }

    pub(super) fn save_avatar(&self, id: EndpointId, bytes: &[u8]) -> Result<()> {
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
    pub(super) fn publish(&self) -> Result<()> {
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
    let open = crate::platform::OnClick::OpenChat(f.code.clone());
    crate::platform::notify(&format!("{} is online", f.nickname), &body, f.avatar.as_deref(), false, open);
}

impl Shared {
    /// Nickname of a friend (empty if unknown).
    pub(super) fn nickname(&self, id: &EndpointId) -> String {
        self.friends.lock().unwrap().get(id).map(|f| f.nickname.clone()).unwrap_or_default()
    }

    /// Friend id, code and nickname from a nickname or a code.
    pub(super) fn resolve(&self, who: &str) -> Option<(EndpointId, String, String)> {
        FriendList::load(&self.paths)
            .ok()?
            .by_id()
            .into_iter()
            .find_map(|(id, f)| (f.nickname == who || f.code == who).then(|| (id, f.code.clone(), f.nickname.clone())))
    }
}
