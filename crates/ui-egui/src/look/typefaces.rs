//! Typefaces the message editor draws with (Arial, Calibri, Times New Roman…): each one the
//! system has is read once, asked for by name, and becomes an egui family named after it (and
//! `<name>-bold` for its bold), with the usual fonts after it for the characters it lacks. One
//! the system hasn't isn't asked for again: the editor draws it in the usual font, as before,
//! and the message is still sent with the name.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{FontData, FontFamily};

/// What's been asked for, and what's there, kept with the context (a change of the usual
/// fonts starts afresh: [`forget`]).
#[derive(Clone, Default)]
struct State {
    tried: BTreeSet<String>,
    loaded: BTreeSet<String>,
}

fn id() -> egui::Id {
    egui::Id::unique("quatrelle-typefaces")
}

fn state(ctx: &egui::Context) -> State {
    ctx.data(|d| d.get_temp::<State>(id())).unwrap_or_default()
}

/// The typefaces loaded, by name: those the editor can draw with.
pub(crate) fn loaded(ctx: &egui::Context) -> BTreeSet<String> {
    state(ctx).loaded
}

/// The family to draw `name` in (bold or not), when it's loaded.
pub(crate) fn family(loaded: &BTreeSet<String>, name: &str, bold: bool) -> Option<FontFamily> {
    loaded.contains(name).then(|| FontFamily::Name(if bold { format!("{name}-bold").into() } else { name.to_string().into() }))
}

/// Of the typefaces `names`, those not asked for yet (each once).
pub(crate) fn wanted(ctx: &egui::Context, names: impl IntoIterator<Item = String>) -> Vec<String> {
    let st = state(ctx);
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if !n.is_empty() && !st.tried.contains(&n) && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Read `name` from the system (once) and make it a family of its own, and its bold another,
/// the usual fonts after each.
pub(crate) fn load(ctx: &egui::Context, services: &mut dyn crate::Services, name: &str) {
    let mut st = state(ctx);
    if st.tried.contains(name) {
        return;
    }
    // The fonts in place, to add to: none before the context's first frame (egui then has
    // nothing to give), in which case it's asked again at a later frame.
    let Some(mut defs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.fonts(|f| f.definitions().clone()))).ok() else { return };
    st.tried.insert(name.to_string());
    ctx.data_mut(|d| d.insert_temp(id(), st.clone()));
    let Some((regular, bold)) = services.typeface(name) else { return };
    // A file egui can't read stops the window when the typeface is drawn with: it's drawn in
    // the usual font instead (fontconfig's Symbol on Linux is a PostScript Type 1 file, and a
    // message written in it closed the window, 29/10).
    if !readable(&format!("the typeface {name}"), &regular, 0) {
        return;
    }
    let bold = bold.filter(|b| readable(&format!("the typeface {name} (bold)"), b, 0));
    let usual: Vec<String> = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let usual_bold: Vec<String> = defs.families.get(&FontFamily::Name("bold".into())).cloned().unwrap_or_else(|| usual.clone());
    let key = format!("typeface-{name}");
    defs.font_data.insert(key.clone(), Arc::new(FontData::from_owned(regular)));
    let mut list = vec![key.clone()];
    list.extend(usual);
    defs.families.insert(FontFamily::Name(name.to_string().into()), list);
    let bold_key = match bold {
        Some(bytes) => {
            let k = format!("typeface-{name}-bold");
            defs.font_data.insert(k.clone(), Arc::new(FontData::from_owned(bytes)));
            k
        }
        None => key,
    };
    let mut list = vec![bold_key];
    list.extend(usual_bold);
    defs.families.insert(FontFamily::Name(format!("{name}-bold").into()), list);
    ctx.set_fonts(defs);
    st.loaded.insert(name.to_string());
    ctx.data_mut(|d| d.insert_temp(id(), st));
}

/// Whether egui can read `bytes` as a font (the face at `index` of a collection), said in the
/// log when it can't (`what`: which font it is). egui reads TrueType and OpenType, as skrifa
/// does here, and stops at the next frame on anything else.
pub(crate) fn readable(what: &str, bytes: &[u8], index: u32) -> bool {
    match skrifa::FontRef::from_index(bytes, index) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("{what}: not a font Quatrelle can draw with ({e}), left out");
            false
        }
    }
}

/// The usual fonts were installed anew (the desktop's font changed): what was loaded went with
/// them, and is asked for again when needed.
pub(crate) fn forget(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<State>(id()));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fonts;
    impl crate::Services for Fonts {
        fn clock(&self) -> (i64, i32) {
            (0, 0)
        }
        fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
        fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
        fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
            Ok(())
        }
        fn typeface(&mut self, family: &str) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
            // egui's own font stands in for Arial; Symbol is a PostScript Type 1 file, as
            // fontconfig gives on Linux; nothing else is installed.
            match family {
                "Arial" => Some((
                    egui::FontDefinitions::default().font_data.values().next().map(|d| d.font.as_ref().as_ref().to_vec()).unwrap_or_default(),
                    None,
                )),
                "Symbol" => Some((b"%!PS-AdobeFont-1.0: StandardSymbolsPS 1.00".to_vec(), Some(b"%!PS".to_vec()))),
                _ => None,
            }
        }
    }

    /// A typeface the system has becomes a family the editor can draw with, asked for once;
    /// one it hasn't is left alone (and not asked for again).
    #[test]
    fn a_typeface_is_loaded_once_and_drawn_with() {
        let ctx = egui::Context::default();
        let mut services = Fonts;
        assert_eq!(wanted(&ctx, ["Arial".to_string(), "Arial".to_string(), String::new(), "Nope".to_string()]), ["Arial", "Nope"]);
        // Fonts exist from the first frame on (before it, `load` leaves everything be).
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        load(&ctx, &mut services, "Arial");
        load(&ctx, &mut services, "Nope");
        let have = loaded(&ctx);
        assert!(have.contains("Arial") && !have.contains("Nope"), "{have:?}");
        assert!(wanted(&ctx, ["Arial".to_string(), "Nope".to_string()]).is_empty(), "asked for once");
        assert_eq!(family(&have, "Arial", true), Some(FontFamily::Name("Arial-bold".into())));
        assert_eq!(family(&have, "Nope", false), None);
        // Drawn with: the family is bound, so laying text out in it doesn't fail.
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let g =
                ui.fonts_mut(|f| f.layout_no_wrap("Hello".into(), egui::FontId::new(14.0, FontFamily::Name("Arial".into())), egui::Color32::WHITE));
            assert!(g.size().x > 0.0);
        });
        out.textures_delta.clear();
        forget(&ctx);
        assert!(loaded(&ctx).is_empty());
    }

    /// A typeface the system gives as a file egui can't read is left out: the editor draws it
    /// in the usual font, and the window goes on (a message written in Symbol closed it, 29/10).
    #[test]
    fn a_typeface_egui_cant_read_is_left_out() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        load(&ctx, &mut Fonts, "Symbol");
        let have = loaded(&ctx);
        assert!(!have.contains("Symbol"), "{have:?}");
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for family in [family(&have, "Symbol", false), family(&have, "Symbol", true), Some(FontFamily::Proportional)].into_iter().flatten() {
                let g = ui.fonts_mut(|f| f.layout_no_wrap("Hello".into(), egui::FontId::new(14.0, family), egui::Color32::WHITE));
                assert!(g.size().x > 0.0);
            }
        });
        out.textures_delta.clear();
    }
}
