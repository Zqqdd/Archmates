//! What happens when a friend sends a file (asks first, then saves it) or a voice message.

use std::path::PathBuf;
use std::sync::Arc;

use iroh::EndpointId;

use super::state::{Offer, PendingOffer, Shared};
use crate::common::chat;
use crate::common::time::now_ms;
use crate::common::transfer::{FileReceiver, OfferFn, SavedFn};
use crate::common::voice::{ReceivedFn, VoiceReceiver};

/// Files: shows the offer (notification + widgets), waits for your answer, records the file.
pub(super) fn files(shared: Arc<Shared>) -> FileReceiver {
    let on_offer: Arc<OfferFn> = {
        let shared = shared.clone();
        Arc::new(move |id: EndpointId, name: String, size: u64| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let offer = Offer { id: chat::new_id(), name: name.clone(), size };
            let nickname = shared.nickname(&id);
            shared.offers.lock().unwrap().insert(
                offer.id.clone(),
                PendingOffer { friend: id, offer: offer.clone(), answer: tx, notification: None },
            );
            let _ = shared.publish();
            // Notification with Accept / Decline buttons (the widgets show the same buttons).
            let (shared, offer_id) = (shared.clone(), offer.id.clone());
            std::thread::spawn(move || {
                let title = format!("{nickname} wants to send you a file");
                let body = format!("{name} ({:.1} MB)", size as f64 / 1_048_576.0);
                let shown = |n| shared.set_offer_notification(&offer_id, n);
                if let Some(accept) = crate::platform::ask(&title, &body, shown) {
                    shared.answer_offer(&offer_id, accept);
                }
            });
            rx
        })
    };
    let on_saved: Arc<SavedFn> = {
        Arc::new(move |id: EndpointId, name: String, saved: PathBuf| {
            let code = id.to_string();
            let nickname = shared.nickname(&id);
            let message = chat::Message {
                id: chat::new_id(),
                dir: "in".into(),
                text: format!("📎 {name}"),
                ms: now_ms(),
                delivered: true,
                file: Some(saved.to_string_lossy().into_owned()),
                voice: None,
                received_ms: None,
            };
            let _ = chat::append(&shared.paths, &code, &message);
            crate::platform::notify(
                &format!("File received from {nickname}"),
                &format!("{name}\nSaved in {}", saved.parent().map(|p| p.display().to_string()).unwrap_or_default()),
                None,
                true,
                crate::platform::OnClick::OpenPath(
                    saved.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| saved.clone()),
                ),
            );
            let _ = shared.publish();
        })
    };
    FileReceiver { on_offer, on_saved }
}

/// Voice messages: recorded in the conversation, with a notification.
pub(super) fn voice(shared: Arc<Shared>) -> VoiceReceiver {
    let on_received: Arc<ReceivedFn> =
        Arc::new(move |id: EndpointId, msg_id: String, secs: u32, saved: PathBuf, ms: u64| {
            let code = id.to_string();
            let nickname = shared.nickname(&id);
            let message = chat::Message {
                id: msg_id,
                dir: "in".into(),
                text: format!("🎤 Voice message ({})", crate::common::voice::format_secs(secs)),
                ms,
                delivered: true,
                file: Some(saved.to_string_lossy().into_owned()),
                voice: Some(secs),
                received_ms: None,
            };
            if chat::receive(&shared.paths, &code, message).unwrap_or(false) {
                crate::platform::notify(
                    &nickname,
                    &format!("🎤 Voice message ({})", crate::common::voice::format_secs(secs)),
                    None,
                    true,
                    crate::platform::OnClick::OpenChat(code.clone()),
                );
            }
            let _ = shared.publish();
        });
    VoiceReceiver { on_received }
}
