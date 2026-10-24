# Third-party assets

Every non-code asset in this repository has a row here (path, author, source, licence).
`cargo xtask assets` (part of `cargo xtask ci`) fails otherwise.

The interface icons are Microsoft's Fluent UI System Icons
([microsoft/fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons), from the
`@fluentui/svg-icons` 1.1.292 package), MIT licence, copyright (c) 2020 Microsoft Corporation; the
licence is `assets/icons/fluent/LICENSE.txt`. Otherwise Quatrelle ships no third-party assets: the sample picture is generated in code, and the UI uses an
installed system font when one is found. Builds made with `CRAFT_FONTS_DIR` (the releases) embed
the fonts listed in [storytold/craft-fonts](https://github.com/storytold/craft-fonts)' `ATTRIBUTION.md`
(SIL Open Font License 1.1); none are in this repository, and each one's licence is shown on the
About page. On Linux, the title bar drawn under GNOME's Wayland session (winit's
`wayland-csd-adwaita`, from [sctk-adwaita](https://github.com/PolyMeilex/sctk-adwaita), MIT)
writes the title in the desktop's title-bar font, found with `fc-match`; only when there is none
does it fall back to the Cantarell Regular that crate embeds (SIL Open Font License 1.1). The app icon files in `assets/app-icon/`
are rendered by `cargo xtask icons` from Quatrelle's own drawing in `xtask/src/icons.rs`, or from the
owner's own picture `assets/icon.png` when it is there (every size is sampled down from it).

| Path | Author | Source | Licence |
|---|---|---|---|
| `assets/icon.png` | Quatrelle's owner | The picture the app icon is rendered from, when present | Quatrelle's own |
| `docs/images/reading.png` | Quatrelle contributors | Screenshot of Quatrelle's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/search-dark.png` | Quatrelle contributors | Screenshot of Quatrelle's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/compose.png` | Quatrelle contributors | Screenshot of Quatrelle's own sample mailbox | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/128x128/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/16x16/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/24x24/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/256x256/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/32x32/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/48x48/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/512x512/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/64x64/apps/ai.storyteller.quatrelle.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/scalable/apps/ai.storyteller.quatrelle.svg` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/quatrelle-1024.png` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `fuzz/seeds/photo/geotagged.jpg` | Quatrelle contributors | Written byte by byte as a fuzzing seed (a few header bytes, no picture) | MIT OR Apache-2.0 |
| `fuzz/seeds/photo/geotagged.png` | Quatrelle contributors | Written byte by byte as a fuzzing seed (a few header bytes, no picture) | MIT OR Apache-2.0 |
| `assets/app-icon/quatrelle.icns` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/quatrelle.ico` | Quatrelle contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/dictionaries/LICENSE.txt` | Quatrelle contributors | What the dictionaries below are licensed under, and where they come from | MIT OR Apache-2.0 |
| `assets/dictionaries/MPL-2.0.txt` | Mozilla | [Mozilla Public License 2.0](https://www.mozilla.org/media/MPL/2.0/index.txt), unchanged | MPL-2.0 |
| `assets/dictionaries/en_US.aff.gz` | Kevin Atkinson / SCOWL | [SCOWL](http://wordlist.aspell.net/) (`hunspell-en-us`), gzipped unchanged | MIT-like / BSD-like / Public domain |
| `assets/dictionaries/en_US.dic.gz` | Kevin Atkinson / SCOWL | [SCOWL](http://wordlist.aspell.net/) (`hunspell-en-us`), gzipped unchanged | MIT-like / BSD-like / Public domain |
| `assets/dictionaries/fr_FR.aff.gz` | Dicollecte | [Dicollecte](https://www.dicollecte.org/) (`hunspell-fr`), gzipped unchanged | MPL-2.0 (of its MPL/LGPL/GPL triple licence) |
| `assets/dictionaries/fr_FR.dic.gz` | Dicollecte | [Dicollecte](https://www.dicollecte.org/) (`hunspell-fr`), gzipped unchanged | MPL-2.0 (of its MPL/LGPL/GPL triple licence) |
| `assets/icons/fluent/LICENSE.txt` | Microsoft Corporation | [fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons/blob/main/LICENSE) (the licence of the icons beside it) | MIT |
| `assets/icons/fluent/add_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/alert_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/archive_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_enter_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_forward_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_left_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_reply_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_reply_all_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_sync_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_undo_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/attach_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/book_contacts_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_ltr_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/call_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/chevron_down_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/chevron_left_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/chevron_right_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/clock_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/delete_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/dismiss_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/document_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/drafts_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/filter_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/flag_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/folder_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/folder_arrow_right_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/image_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/info_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/layout_column_two_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/link_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/local_language_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/location_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_add_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_inbox_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_inbox_arrow_up_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_prohibited_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/flash_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_read_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/note_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/open_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/people_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/person_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/person_clock_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/pin_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/print_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/search_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/send_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/settings_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/star_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/storage_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/tag_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_align_center_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_align_left_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_align_right_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_bullet_list_ltr_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_clear_formatting_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_grammar_checkmark_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_indent_decrease_ltr_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_indent_increase_ltr_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_number_list_ltr_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/warning_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/weather_moon_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/weather_sunny_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_day_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_work_week_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_month_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_today_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_week_start_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/chevron_up_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_add_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/vote_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/arrow_down_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/calendar_clock_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/clipboard_paste_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/copy_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/cut_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/eye_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/font_decrease_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/font_increase_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/important_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/line_horizontal_1_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/person_available_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/signature_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/text_word_count_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
