// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pictures the Teams worker sends (images kept with a message, people's photos, images from
//! elsewhere the user clicked), decoded on a thread of their own ([`Decoder`]) and kept as
//! textures: photos by user for the session, images by picture at a thumbnail's size within a
//! budget of bytes (the least recently shown let go, asked for again once back on screen), and
//! the one opened big decoded whole only then. The engine's
//! queue of them is emptied every frame, whatever the module (`TeamsQueue::apply`), and what's
//! decoded becomes textures every frame (`MailApp::logic`).
//!
//! What is hostile here: the bytes (a sender chose them, or their server did). They're decoded
//! off the window's thread, behind `catch_unwind`, within [`MAX_PIXELS`] pixels and
//! [`MAX_ALLOC`] bytes ([`decode_bounded`]); one past those, or one that panics the decoder, is a
//! placeholder ([`Shown::Failed`]) and the thread goes on. How much: at most [`WAITING_MAX`]
//! wait, [`CHANNEL`] are with the thread, [`IMAGES_BYTES`] of images' textures (each at most
//! [`THUMB_SIDE`] on a side) and [`SOURCES_BYTES`] of their files are kept, [`PHOTOS_MAX`]
//! photos, and one picture whole ([`WHOLE_SIDE`]) while the viewer shows it.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};

use egui::{ColorImage, TextureHandle, TextureOptions};
use quatrelle_engine::Session;
use quatrelle_engine::model::pim::teams::Picture;

use crate::reading::pictures::decode_bounded;

/// The most pixels a picture may have, and the most bytes its decoding may take.
pub(crate) const MAX_PIXELS: u64 = 40_000_000;
pub(crate) const MAX_ALLOC: u64 = 64 << 20;
/// The pictures handed to the thread at once, and its answers waiting.
pub(crate) const CHANNEL: usize = 32;
/// The longest side an image is kept at (its thumbnail, sharp at twice its size), one opened in
/// the viewer, and a photo.
const THUMB_SIDE: u32 = 480;
const WHOLE_SIDE: u32 = 1600;
const PHOTO_SIDE: u32 = 96;
/// The most pictures waiting for the thread: past it the oldest is given up (a placeholder).
const WAITING_MAX: usize = 64;
/// The most bytes of images' textures kept, and of their files (to open them big): past it the
/// least recently shown go (never one shown in the last two frames). The most photos.
pub(crate) const IMAGES_BYTES: usize = 48 << 20;
const SOURCES_BYTES: usize = 32 << 20;
const PHOTOS_MAX: usize = 2_000;
/// The most pictures remembered as let go, until the pane asks for them again.
const LET_GO_MAX: usize = 1_000;
/// The most textures made in a frame.
const MADE_A_FRAME: usize = 8;

/// How a picture's bytes become pixels, at most `side` on a side; `None`: not a picture.
pub(crate) type Decode = fn(&[u8], u32) -> Option<ColorImage>;

/// A picture's pixels: decoded within the limits, shrunk to `side`.
pub(crate) fn pixels(data: &[u8], side: u32) -> Option<ColorImage> {
    let img = decode_bounded(data, MAX_PIXELS, MAX_ALLOC)?;
    let img = if img.width() > side || img.height() > side { img.thumbnail(side, side) } else { img }.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// What became of an image: its texture, or a placeholder (it couldn't be shown).
#[derive(Clone)]
pub(crate) enum Shown {
    Ready(TextureHandle),
    Failed,
}

/// A picture for the thread: its bytes, its longest side, and whether it's the one opened big.
struct Work {
    picture: Picture,
    bytes: Arc<[u8]>,
    side: u32,
    whole: bool,
}

type Done = (Picture, bool, Option<ColorImage>);

/// One thread that decodes pictures, started on the first, fed through a channel of
/// [`CHANNEL`]. A decoding that panics answers `None`; the thread goes on.
pub(crate) struct Decoder {
    decode: Decode,
    to: Option<SyncSender<Work>>,
    from: Option<Receiver<Done>>,
    /// Pictures with the thread, not answered yet.
    out: usize,
    /// The thread couldn't start (the browser has none): every picture is a placeholder.
    no_thread: bool,
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new(pixels)
    }
}

impl Decoder {
    pub(crate) fn new(decode: Decode) -> Decoder {
        Decoder { decode, to: None, from: None, out: 0, no_thread: false }
    }

    /// The thread, started now if it isn't yet; whether there is one.
    fn started(&mut self, ctx: &egui::Context) -> bool {
        if self.to.is_some() {
            return true;
        }
        if self.no_thread {
            return false;
        }
        let (to, work) = sync_channel::<Work>(CHANNEL);
        let (done, from) = sync_channel::<Done>(CHANNEL);
        let (decode, ctx) = (self.decode, ctx.clone());
        let started = std::thread::Builder::new().name("teams-pictures".into()).spawn(move || {
            for w in work {
                let pixels = std::panic::catch_unwind(|| decode(&w.bytes, w.side)).ok().flatten();
                if done.send((w.picture, w.whole, pixels)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        match started {
            Ok(_) => {
                self.to = Some(to);
                self.from = Some(from);
                self.out = 0;
                true
            }
            Err(e) => {
                log::warn!("teams: the pictures' thread didn't start ({e}); pictures aren't shown");
                self.no_thread = true;
                false
            }
        }
    }

    /// `work` handed to the thread; given back when its queue is full.
    fn send(&mut self, ctx: &egui::Context, work: Work) -> Result<(), Option<Work>> {
        if !self.started(ctx) {
            return Err(None);
        }
        let Some(to) = &self.to else { return Err(Some(work)) };
        match to.try_send(work) {
            Ok(()) => {
                self.out = self.out.saturating_add(1);
                Ok(())
            }
            Err(TrySendError::Full(work)) => Err(Some(work)),
            // The thread is gone: started again for the next.
            Err(TrySendError::Disconnected(work)) => {
                self.stopped();
                Err(Some(work))
            }
        }
    }

    /// An answer of the thread, if one is in.
    fn answer(&mut self) -> Option<Done> {
        let from = self.from.as_ref()?;
        match from.try_recv() {
            Ok(done) => {
                self.out = self.out.saturating_sub(1);
                Some(done)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.stopped();
                None
            }
        }
    }

    fn stopped(&mut self) {
        self.to = None;
        self.from = None;
        self.out = 0;
    }

    /// Whether its thread is running.
    #[cfg(test)]
    pub(crate) fn alive(&self) -> bool {
        self.to.is_some()
    }
}

/// An image kept: what became of it, its texture's bytes, and the frame it was last shown in.
struct Image {
    shown: Shown,
    bytes: usize,
    seen: Cell<u64>,
}

/// The Teams pictures: those waiting, and those decoded, as textures.
#[derive(Default)]
pub(crate) struct TeamsPictures {
    pub(crate) decoder: Decoder,
    waiting: VecDeque<Work>,
    /// Images (kept with a message, or from elsewhere), and their textures' bytes together.
    images: HashMap<Picture, Image>,
    held: usize,
    /// Their files, to open them big, oldest first, and their bytes together.
    sources: HashMap<Picture, Arc<[u8]>>,
    sources_order: VecDeque<Picture>,
    sources_held: usize,
    /// Images let go for room, for the pane to ask for again once they're on screen.
    let_go: Vec<Picture>,
    /// The picture the viewer shows, and its whole texture once decoded; whether its decoding
    /// was asked for, and whether it was asked of the worker again (its file let go).
    whole: Option<(Picture, Option<TextureHandle>)>,
    whole_sent: bool,
    whole_asked: bool,
    /// Photos by user, for the session; a photo that couldn't be shown isn't kept (the initials
    /// stay).
    photos: HashMap<String, TextureHandle>,
    /// The frames made so far.
    frame: u64,
    /// The account and the chat they were for.
    account: String,
    open: Option<String>,
}

impl TeamsPictures {
    /// The pictures the engine holds taken (every frame, whatever the module): another account
    /// signed in, nothing of the last one's is kept; another chat opened, its images go (the
    /// engine asks them again when it's opened again).
    pub(crate) fn take(&mut self, s: &mut Session) {
        if self.account != s.prefs.teams.object_id {
            self.account.clone_from(&s.prefs.teams.object_id);
            self.waiting.clear();
            self.forget(|_| true);
            self.photos.clear();
        }
        if self.open != s.teams.open {
            self.open.clone_from(&s.teams.open);
            let open = self.open.clone();
            let other = move |p: &Picture| matches!(p, Picture::Hosted { chat, .. } if Some(chat) != open.as_ref());
            self.waiting.retain(|w| !other(&w.picture));
            self.forget(other);
        }
        for (picture, bytes) in s.teams.take_pictures() {
            let bytes: Arc<[u8]> = bytes.into();
            let side = if matches!(picture, Picture::Photo { .. }) {
                PHOTO_SIDE
            } else {
                self.keep_source(&picture, bytes.clone());
                THUMB_SIDE
            };
            self.waiting.push_back(Work { picture, bytes, side, whole: false });
        }
        while self.waiting.len() > WAITING_MAX {
            if let Some(w) = self.waiting.pop_front() {
                self.keep(w.picture, Shown::Failed, 0);
            }
        }
    }

    /// The images `gone` takes let go, with their files and the whole one.
    fn forget(&mut self, gone: impl Fn(&Picture) -> bool) {
        self.images.retain(|p, _| !gone(p));
        self.held = self.images.values().map(|i| i.bytes).sum();
        self.sources.retain(|p, _| !gone(p));
        self.sources_order.retain(|p| !gone(p));
        self.sources_held = self.sources.values().map(|b| b.len()).sum();
        self.let_go.retain(|p| !gone(p));
        if self.whole.as_ref().is_some_and(|(p, _)| gone(p)) {
            self.whole = None;
        }
    }

    /// An image's file kept (to open it big), the oldest let go past [`SOURCES_BYTES`].
    fn keep_source(&mut self, picture: &Picture, bytes: Arc<[u8]>) {
        if let Some(old) = self.sources.insert(picture.clone(), bytes.clone()) {
            self.sources_held = self.sources_held.saturating_sub(old.len());
        } else {
            self.sources_order.push_back(picture.clone());
        }
        self.sources_held = self.sources_held.saturating_add(bytes.len());
        while self.sources_held > SOURCES_BYTES {
            let Some(old) = self.sources_order.pop_front() else { break };
            if let Some(b) = self.sources.remove(&old) {
                self.sources_held = self.sources_held.saturating_sub(b.len());
            }
        }
    }

    /// The pictures waiting handed to the thread, and what it decoded made into textures (a few
    /// a frame). Every frame.
    pub(crate) fn made(&mut self, ctx: &egui::Context) {
        self.frame = self.frame.saturating_add(1);
        self.ask_whole();
        while let Some(work) = self.waiting.pop_front() {
            match self.decoder.send(ctx, work) {
                Ok(()) => {}
                Err(Some(work)) => {
                    self.waiting.push_front(work);
                    break;
                }
                Err(None) => {
                    // No thread: none can be shown.
                    let failed: Vec<Picture> = self.waiting.drain(..).filter(|w| !w.whole).map(|w| w.picture).collect();
                    for p in failed {
                        self.keep(p, Shown::Failed, 0);
                    }
                    break;
                }
            }
        }
        for _ in 0..MADE_A_FRAME {
            let Some((picture, whole, pixels)) = self.decoder.answer() else { break };
            // Another account's, or another chat's: the thread was still at it.
            if !self.wanted(&picture) {
                continue;
            }
            let texture = pixels.map(|ci| {
                let bytes = ci.size[0].saturating_mul(ci.size[1]).saturating_mul(4);
                let filter = if ci.size[0].max(ci.size[1]) < 96 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
                (ctx.load_texture("teams-picture", ci, filter), bytes)
            });
            if whole {
                if let Some((p, tex)) = &mut self.whole
                    && *p == picture
                {
                    *tex = texture.map(|(t, _)| t);
                }
                continue;
            }
            match texture {
                Some((tex, bytes)) => self.keep(picture, Shown::Ready(tex), bytes),
                None => self.keep(picture, Shown::Failed, 0),
            }
        }
        if !self.waiting.is_empty() || self.decoder.out > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// The picture the viewer shows decoded whole, from its file, once.
    fn ask_whole(&mut self) {
        let Some((picture, None)) = &self.whole else { return };
        if self.whole_sent {
            return;
        }
        let Some(bytes) = self.sources.get(picture).cloned() else { return };
        self.whole_sent = true;
        self.waiting.push_front(Work { picture: picture.clone(), bytes, side: WHOLE_SIDE, whole: true });
    }

    /// Whether a picture decoded is still wanted: a photo of this account's, an image of the open
    /// chat or from elsewhere.
    fn wanted(&self, p: &Picture) -> bool {
        match p {
            Picture::Hosted { chat, .. } => self.open.as_deref() == Some(chat.as_str()),
            Picture::Photo { .. } | Picture::Outside { .. } => true,
        }
    }

    fn keep(&mut self, picture: Picture, shown: Shown, bytes: usize) {
        if let Picture::Photo { user } = picture {
            // Past the most kept, a new one's person keeps their initials.
            let Shown::Ready(tex) = shown else { return };
            if self.photos.len() < PHOTOS_MAX || self.photos.contains_key(&user) {
                self.photos.insert(user, tex);
            }
            return;
        }
        let image = Image { shown, bytes, seen: Cell::new(self.frame) };
        if let Some(old) = self.images.insert(picture, image) {
            self.held = self.held.saturating_sub(old.bytes);
        }
        self.held = self.held.saturating_add(bytes);
        // Past the budget, the least recently shown go; those on screen stay.
        while self.held > IMAGES_BYTES {
            let recent = self.frame.saturating_sub(2);
            let oldest = self.images.iter().filter(|(_, i)| i.seen.get() < recent).min_by_key(|(_, i)| i.seen.get()).map(|(p, _)| p.clone());
            let Some(old) = oldest else { break };
            if let Some(i) = self.images.remove(&old) {
                self.held = self.held.saturating_sub(i.bytes);
            }
            if let Some(b) = self.sources.remove(&old) {
                self.sources_held = self.sources_held.saturating_sub(b.len());
                self.sources_order.retain(|p| *p != old);
            }
            if self.let_go.len() >= LET_GO_MAX {
                self.let_go.clear();
            }
            self.let_go.push(old);
        }
    }

    /// The images let go for room since last asked: the pane asks for them again once on screen.
    pub(crate) fn take_let_go(&mut self) -> Vec<Picture> {
        std::mem::take(&mut self.let_go)
    }

    /// The picture the viewer shows (none: closed). Whether it has to be asked of the worker
    /// again (its file was let go), once.
    pub(crate) fn view(&mut self, picture: Option<&Picture>) -> bool {
        if self.whole.as_ref().map(|(p, _)| p) != picture {
            self.whole = picture.map(|p| (p.clone(), None));
            self.whole_sent = false;
            self.whole_asked = false;
        }
        let missing = picture.is_some_and(|p| !self.sources.contains_key(p));
        if missing && !self.whole_asked {
            self.whole_asked = true;
            return true;
        }
        if !missing {
            // Asked again and come back: it can be decoded whole now.
            self.whole_asked = false;
        }
        false
    }

    /// The picture the viewer shows, whole, once decoded.
    pub(crate) fn whole(&self, picture: &Picture) -> Option<&TextureHandle> {
        self.whole.as_ref().filter(|(p, _)| p == picture).and_then(|(_, t)| t.as_ref())
    }

    /// An image as it stands (none while it's waited for, or once let go); it counts as shown in
    /// this frame.
    pub(crate) fn image(&self, p: &Picture) -> Option<&Shown> {
        let image = self.images.get(p)?;
        image.seen.set(self.frame);
        Some(&image.shown)
    }

    /// A person's photo, once it came.
    pub(crate) fn photo(&self, user: &str) -> Option<&TextureHandle> {
        self.photos.get(user)
    }

    /// Whether some pictures wait for the thread or are with it.
    #[cfg(test)]
    pub(crate) fn busy(&self) -> bool {
        !self.waiting.is_empty() || self.decoder.out > 0
    }
}
