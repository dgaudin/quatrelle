// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The order EWS's schema gives an appointment's and a task's fields, which the fake checks a new
//! one against.

use super::*;

/// An appointment's fields as EWS's schema orders them (those Quatrelle writes): the item's
/// (`ItemType`), then the appointment's (`CalendarItemType`).
const CALENDAR_ITEM_ORDER: &[&str] = &[
    "ItemClass",
    "Subject",
    "Sensitivity",
    "Body",
    "Categories",
    "Importance",
    "ReminderIsSet",
    "ReminderMinutesBeforeStart",
    "ExtendedProperty",
    "UID",
    "Start",
    "End",
    "IsAllDayEvent",
    "LegacyFreeBusyStatus",
    "Location",
    "IsResponseRequested",
    "RequiredAttendees",
    "OptionalAttendees",
    "Resources",
    "Recurrence",
    "MeetingTimeZone",
    "StartTimeZone",
    "EndTimeZone",
    "AllowNewTimeProposal",
];

/// A task's fields as EWS's schema orders them (those Quatrelle writes): the item's, then the
/// task's (`TaskType`).
const TASK_ORDER: &[&str] = &[
    "ItemClass",
    "Subject",
    "Sensitivity",
    "Body",
    "Categories",
    "Importance",
    "ReminderDueBy",
    "ReminderIsSet",
    "CompleteDate",
    "DueDate",
    "PercentComplete",
    "StartDate",
    "Status",
];

/// Whether a new appointment's or task's fields come in the schema's order (one it doesn't
/// list is let be).
pub(crate) fn in_schema_order(item: &El) -> bool {
    let order = if item.name == "Task" { TASK_ORDER } else { CALENDAR_ITEM_ORDER };
    let places: Vec<usize> = item.kids.iter().filter_map(|k| order.iter().position(|n| *n == k.name)).collect();
    places.windows(2).all(|w| w.first() <= w.get(1))
}
