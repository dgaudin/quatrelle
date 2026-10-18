# Third-party assets

Every non-code asset in this repository has a row here (path, author, source, licence).
`cargo xtask assets` (part of `cargo xtask ci`) fails otherwise.

The interface icons are Microsoft's Fluent UI System Icons
([microsoft/fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons), from the
`@fluentui/svg-icons` 1.1.292 package), MIT licence, copyright (c) 2020 Microsoft Corporation; the
licence is `assets/icons/fluent/LICENSE.txt`. Otherwise MailCraft ships no third-party assets: the sample picture is generated in code, and the UI uses an
installed system font when one is found. Builds made with `CRAFT_FONTS_DIR` (the releases) embed
the fonts listed in [storytold/craft-fonts](https://github.com/storytold/craft-fonts)' `ATTRIBUTION.md`
(SIL Open Font License 1.1); none are in this repository, and each one's licence is shown on the
About page. On Linux, the title bar drawn under GNOME's Wayland session (winit's
`wayland-csd-adwaita`, from [sctk-adwaita](https://github.com/PolyMeilex/sctk-adwaita), MIT)
writes the title in the desktop's title-bar font, found with `fc-match`; only when there is none
does it fall back to the Cantarell Regular that crate embeds (SIL Open Font License 1.1). The app icon files in `assets/app-icon/`
are rendered by `cargo xtask icons` from MailCraft's own drawing in `xtask/src/icons.rs`.

| Path | Author | Source | Licence |
|---|---|---|---|
| `docs/images/reading.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/search-dark.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `docs/images/compose.png` | MailCraft contributors | Screenshot of MailCraft's own sample mailbox | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/128x128/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/16x16/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/24x24/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/256x256/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/32x32/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/48x48/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/512x512/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/64x64/apps/ai.storyteller.mailcraft.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/scalable/apps/ai.storyteller.mailcraft.svg` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft-1024.png` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft.icns` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/app-icon/mailcraft.ico` | MailCraft contributors | Rendered by `cargo xtask icons` (`xtask/src/icons.rs`) | MIT OR Apache-2.0 |
| `assets/icons/fluent/LICENSE.txt` | Microsoft Corporation | [fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons/blob/main/LICENSE) (the licence of the icons beside it) | MIT |
| `assets/icons/fluent/add_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
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
| `assets/icons/fluent/location_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_add_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_inbox_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_inbox_arrow_up_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
| `assets/icons/fluent/mail_prohibited_20_regular.svg` | Microsoft Corporation | Fluent UI System Icons, `@fluentui/svg-icons` 1.1.292, unchanged | MIT |
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
