//! The voice desk (see common/voice/desk.rs): the service records and plays voice messages for
//! every front-end, and publishes what it is doing in `voice-desk.json`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::Endpoint;

use super::state::Shared;
use crate::common::config::Paths;
use crate::common::ipc::Answer;
use crate::common::time::now_ms;
use crate::common::voice::desk::{DRAFT_ID, DeskState, DeskStatus, Playing, status_path};

#[derive(Default)]
pub(super) struct Desk {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    to: Option<String>,
    #[cfg(feature = "voice")]
    recorder: Option<crate::common::voice::Recorder>,
    /// The recording under review: (file, seconds).
    draft: Option<(std::path::PathBuf, u32)>,
    sending: bool,
    #[cfg(feature = "voice")]
    player: Option<(String, crate::common::voice::Player)>,
    error: Option<String>,
}

fn ok(message: impl Into<String>) -> Answer {
    Answer { ok: true, message: message.into() }
}

fn fail(message: impl Into<String>) -> Answer {
    Answer { ok: false, message: message.into() }
}

impl Inner {
    fn status(&self) -> DeskStatus {
        #[cfg(feature = "voice")]
        let (recording, secs, level) = match &self.recorder {
            Some(r) => (true, r.elapsed().as_secs_f32(), r.level()),
            None => (false, self.draft.as_ref().map(|d| d.1 as f32).unwrap_or(0.0), 0.0),
        };
        #[cfg(not(feature = "voice"))]
        let (recording, secs, level) = (false, self.draft.as_ref().map(|d| d.1 as f32).unwrap_or(0.0), 0.0);
        #[cfg(feature = "voice")]
        let playing = self.player.as_ref().filter(|(_, p)| !p.is_done()).map(|(id, p)| Playing {
            id: id.clone(),
            position: p.position(),
            secs: p.secs,
        });
        #[cfg(not(feature = "voice"))]
        let playing: Option<Playing> = None;
        let state = if recording {
            DeskState::Recording
        } else if self.sending {
            DeskState::Sending
        } else if self.draft.is_some() {
            DeskState::Review
        } else {
            DeskState::Idle
        };
        DeskStatus {
            state,
            to: if state == DeskState::Idle { None } else { self.to.clone() },
            secs,
            level,
            draft: self.draft.as_ref().map(|d| d.0.to_string_lossy().into_owned()),
            playing,
            error: self.error.clone(),
            ms: now_ms(),
        }
    }

    fn discard(&mut self) {
        #[cfg(feature = "voice")]
        {
            self.recorder = None;
            if self.player.as_ref().is_some_and(|(id, _)| id == DRAFT_ID) {
                self.player = None;
            }
        }
        if let Some((file, _)) = self.draft.take() {
            let _ = std::fs::remove_file(file);
        }
        self.sending = false;
        self.error = None;
    }

    /// Stops the microphone and keeps the recording for review.
    #[cfg(feature = "voice")]
    fn stop(&mut self, paths: &Paths) -> Result<(), String> {
        let Some(recorder) = self.recorder.take() else { return Ok(()) };
        let wav = recorder.finish().map_err(|e| format!("{e:#}"))?;
        let secs = crate::common::voice::wav_secs(&wav).unwrap_or(0);
        if secs == 0 {
            return Err("the recording is empty (is a microphone connected?)".into());
        }
        let file = crate::common::voice::dir(paths).join(format!("draft-{}.wav", now_ms()));
        crate::common::config::write_atomic(&file, &wav).map_err(|e| format!("{e:#}"))?;
        self.draft = Some((file, secs));
        Ok(())
    }
}

impl Desk {
    fn write(&self, paths: &Paths) -> DeskStatus {
        let status = self.inner.lock().unwrap().status();
        if let Ok(json) = serde_json::to_vec(&status) {
            let _ = crate::common::config::write_atomic(&status_path(paths), &json);
        }
        status
    }

    /// Keeps `voice-desk.json` current: ten times a second while recording or playing.
    /// Stops a recording that reached 5 minutes, and forgets a finished playback.
    pub(super) async fn run(shared: Arc<Shared>) {
        let mut last = None;
        loop {
            #[cfg(feature = "voice")]
            {
                let mut inner = shared.voice.inner.lock().unwrap();
                if inner.recorder.as_ref().is_some_and(|r| r.is_full()) {
                    if let Err(e) = inner.stop(&shared.paths) {
                        inner.error = Some(e);
                    }
                }
                if inner.player.as_ref().is_some_and(|(_, p)| p.is_done()) {
                    inner.player = None;
                }
            }
            let status = shared.voice.inner.lock().unwrap().status();
            let busy = matches!(status.state, DeskState::Recording | DeskState::Sending) || status.playing.is_some();
            // Rewritten while busy, else once per change, plus a heartbeat every few seconds
            // so front-ends can tell a stopped service from an idle one.
            let changed = last.as_ref().map(|l: &DeskStatus| DeskStatus { ms: 0, ..l.clone() })
                != Some(DeskStatus { ms: 0, ..status.clone() });
            if busy || changed || last.as_ref().is_none_or(|l| now_ms().saturating_sub(l.ms) > 2_000) {
                last = Some(shared.voice.write(&shared.paths));
            }
            tokio::time::sleep(Duration::from_millis(if busy { 100 } else { 250 })).await;
        }
    }

    pub(super) async fn record(&self, shared: &Shared, to: String) -> Answer {
        let Some((_, code, nickname)) = shared.resolve(&to) else {
            return fail(format!("no friend named \"{to}\""));
        };
        #[cfg(feature = "voice")]
        {
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.sending {
                    return fail("a voice message is being sent: wait a moment");
                }
                inner.discard();
                inner.player = None;
            }
            // Opening the microphone can take a moment: not while holding the lock.
            let recorder = match tokio::task::spawn_blocking(crate::common::voice::Recorder::start).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => return fail(format!("cannot use the microphone: {e:#}")),
                Err(_) => return fail("cannot use the microphone"),
            };
            let mut inner = self.inner.lock().unwrap();
            inner.to = Some(code);
            inner.recorder = Some(recorder);
            drop(inner);
            self.write(&shared.paths);
            ok(format!("recording for {nickname} (5 min max)"))
        }
        #[cfg(not(feature = "voice"))]
        {
            let _ = (code, nickname);
            fail("this archmates was built without voice support")
        }
    }

    pub(super) fn stop(&self, paths: &Paths) -> Answer {
        #[cfg(feature = "voice")]
        {
            let mut inner = self.inner.lock().unwrap();
            let result = inner.stop(paths);
            inner.error = result.as_ref().err().cloned();
            drop(inner);
            let status = self.write(paths);
            match result {
                Ok(()) if status.state == DeskState::Review => {
                    ok(format!("recorded {}", crate::common::voice::format_secs(status.secs as u32)))
                }
                Ok(()) => fail("nothing is being recorded"),
                Err(e) => fail(e),
            }
        }
        #[cfg(not(feature = "voice"))]
        {
            let _ = paths;
            fail("this archmates was built without voice support")
        }
    }

    pub(super) async fn send(&self, endpoint: &Endpoint, shared: &Shared) -> Answer {
        let (file, to) = {
            let mut inner = self.inner.lock().unwrap();
            #[cfg(feature = "voice")]
            if let Err(e) = inner.stop(&shared.paths) {
                inner.error = Some(e.clone());
                return fail(e);
            }
            if inner.sending {
                return fail("already sending");
            }
            let (Some((file, _)), Some(to)) = (inner.draft.clone(), inner.to.clone()) else {
                return fail("there is no recording to send");
            };
            #[cfg(feature = "voice")]
            if inner.player.as_ref().is_some_and(|(id, _)| id == DRAFT_ID) {
                inner.player = None;
            }
            inner.sending = true;
            inner.error = None;
            (file, to)
        };
        self.write(&shared.paths);
        let answer = match std::fs::read(&file) {
            Ok(wav) => super::commands::send_voice(endpoint, shared, &to, &wav).await,
            Err(e) => fail(format!("cannot read the recording: {e}")),
        };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.sending = false;
            if answer.ok {
                inner.discard();
            } else {
                // Kept for review: send again once the friend is back, or discard.
                inner.error = Some(answer.message.clone());
            }
        }
        self.write(&shared.paths);
        answer
    }

    pub(super) fn discard(&self, paths: &Paths) -> Answer {
        self.inner.lock().unwrap().discard();
        self.write(paths);
        ok("recording discarded")
    }

    pub(super) async fn play(&self, paths: &Paths, id: String, path: String) -> Answer {
        #[cfg(feature = "voice")]
        {
            // Only voice files: messages from the voice folder, or the draft.
            let voice_dir = crate::common::voice::dir(paths);
            let inside = std::path::Path::new(&path)
                .canonicalize()
                .ok()
                .zip(voice_dir.canonicalize().ok())
                .is_some_and(|(p, d)| p.starts_with(d));
            if !inside {
                return fail("this voice message has expired");
            }
            self.inner.lock().unwrap().player = None;
            let file = std::path::PathBuf::from(&path);
            let player = match tokio::task::spawn_blocking(move || crate::common::voice::Player::play(&file)).await {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => return fail(format!("cannot play it: {e:#}")),
                Err(_) => return fail("cannot play it"),
            };
            let secs = player.secs;
            self.inner.lock().unwrap().player = Some((id, player));
            self.write(paths);
            ok(format!("playing ({})", crate::common::voice::format_secs(secs as u32)))
        }
        #[cfg(not(feature = "voice"))]
        {
            let _ = (paths, id, path);
            fail("this archmates was built without voice support")
        }
    }

    pub(super) fn stop_playing(&self, paths: &Paths) -> Answer {
        #[cfg(feature = "voice")]
        {
            self.inner.lock().unwrap().player = None;
        }
        self.write(paths);
        ok("stopped")
    }
}
