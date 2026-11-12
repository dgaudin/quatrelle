// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pictures the Teams worker sends (images kept with a message, people's photos, images from
//! elsewhere the user clicked), decoded on a thread of their own ([`Decoder`]) and kept as
//! textures: photos by user for the session, images by picture, a bounded few. The engine's
//! queue of them is emptied every frame, whatever the module (`TeamsQueue::apply`), and what's
//! decoded becomes textures every frame (`MailApp::logic`).
//!
//! What is hostile here: the bytes (a sender chose them, or their server did). They're decoded
//! off the window's thread, behind `catch_unwind`, within [`MAX_PIXELS`] pixels and
//! [`MAX_ALLOC`] bytes ([`decode_bounded`]); one past those, or one that panics the decoder, is a
//! placeholder ([`Shown::Failed`]) and the thread goes on. How many: at most [`WAITING_MAX`]
//! wait, [`CHANNEL`] are with the thread, [`IMAGES_MAX`] images and [`PHOTOS_MAX`] photos are
//! kept.

use std::collections::{HashMap, VecDeque};
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
/// The longest side an image is kept at (its thumbnail, and the viewer), and a photo.
const IMAGE_SIDE: u32 = 1600;
const PHOTO_SIDE: u32 = 96;
/// The most pictures waiting for the thread: past it the oldest is given up (a placeholder).
const WAITING_MAX: usize = 64;
/// The most images and photos kept as textures: past it the oldest go.
const IMAGES_MAX: usize = 64;
const PHOTOS_MAX: usize = 2_000;
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

type Work = (Picture, Vec<u8>, u32);
type Done = (Picture, Option<ColorImage>);

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
            for (picture, bytes, side) in work {
                let pixels = std::panic::catch_unwind(|| decode(&bytes, side)).ok().flatten();
                if done.send((picture, pixels)).is_err() {
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

/// The Teams pictures: those waiting, and those decoded, as textures.
#[derive(Default)]
pub(crate) struct TeamsPictures {
    pub(crate) decoder: Decoder,
    waiting: VecDeque<Work>,
    /// Images (kept with a message, or from elsewhere), oldest first in `order`.
    images: HashMap<Picture, Shown>,
    order: VecDeque<Picture>,
    /// Photos by user, for the session; a photo that couldn't be shown isn't kept (the initials
    /// stay).
    photos: HashMap<String, TextureHandle>,
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
            self.images.clear();
            self.order.clear();
            self.photos.clear();
        }
        if self.open != s.teams.open {
            self.open.clone_from(&s.teams.open);
            let open = self.open.as_deref();
            let other = |p: &Picture| matches!(p, Picture::Hosted { chat, .. } if Some(chat.as_str()) != open);
            self.waiting.retain(|(p, _, _)| !other(p));
            self.images.retain(|p, _| !other(p));
            self.order.retain(|p| !other(p));
        }
        for (picture, bytes) in s.teams.pictures.drain(..) {
            let side = if matches!(picture, Picture::Photo { .. }) { PHOTO_SIDE } else { IMAGE_SIDE };
            self.waiting.push_back((picture, bytes, side));
        }
        while self.waiting.len() > WAITING_MAX {
            if let Some((p, _, _)) = self.waiting.pop_front() {
                self.keep(p, Shown::Failed);
            }
        }
    }

    /// The pictures waiting handed to the thread, and what it decoded made into textures (a few
    /// a frame). Every frame.
    pub(crate) fn made(&mut self, ctx: &egui::Context) {
        while let Some(work) = self.waiting.pop_front() {
            match self.decoder.send(ctx, work) {
                Ok(()) => {}
                Err(Some(work)) => {
                    self.waiting.push_front(work);
                    break;
                }
                Err(None) => {
                    // No thread: none can be shown.
                    let failed: Vec<Picture> = self.waiting.drain(..).map(|(p, _, _)| p).collect();
                    for p in failed {
                        self.keep(p, Shown::Failed);
                    }
                    break;
                }
            }
        }
        for _ in 0..MADE_A_FRAME {
            let Some((picture, pixels)) = self.decoder.answer() else { break };
            // Another account's, or another chat's: the thread was still at it.
            if !self.wanted(&picture) {
                continue;
            }
            let shown = match pixels {
                Some(ci) => {
                    let filter = if ci.size[0].max(ci.size[1]) < 96 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
                    Shown::Ready(ctx.load_texture("teams-picture", ci, filter))
                }
                None => Shown::Failed,
            };
            self.keep(picture, shown);
        }
        if !self.waiting.is_empty() || self.decoder.out > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// Whether a picture decoded is still wanted: a photo of this account's, an image of the open
    /// chat or from elsewhere.
    fn wanted(&self, p: &Picture) -> bool {
        match p {
            Picture::Hosted { chat, .. } => self.open.as_deref() == Some(chat.as_str()),
            Picture::Photo { .. } | Picture::Outside { .. } => true,
        }
    }

    fn keep(&mut self, picture: Picture, shown: Shown) {
        match picture {
            Picture::Photo { user } => {
                // Past the most kept, a new one's person keeps their initials.
                let Shown::Ready(tex) = shown else { return };
                if self.photos.len() < PHOTOS_MAX || self.photos.contains_key(&user) {
                    self.photos.insert(user, tex);
                }
            }
            other => {
                if self.images.insert(other.clone(), shown).is_none() {
                    self.order.push_back(other);
                }
                while self.order.len() > IMAGES_MAX {
                    if let Some(old) = self.order.pop_front() {
                        self.images.remove(&old);
                    }
                }
            }
        }
    }

    /// An image as it stands: none while it's waited for.
    pub(crate) fn image(&self, p: &Picture) -> Option<&Shown> {
        self.images.get(p)
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
