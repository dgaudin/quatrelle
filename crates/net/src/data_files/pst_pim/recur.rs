// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A repeating appointment's pattern (PidLidAppointmentRecur, MS-OXOCAL 2.2.1.44) as an RFC 5545
//! rule and its deleted occurrences.

use super::*;

struct Bytes<'a>(&'a [u8], usize);

impl Bytes<'_> {
    fn u16(&mut self) -> Option<u16> {
        let b = self.0.get(self.1..self.1.checked_add(2)?)?;
        self.1 += 2;
        Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.0.get(self.1..self.1.checked_add(4)?)?;
        self.1 += 4;
        Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
    }
}

const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

fn byday(mask: u32, ordinal: Option<i32>) -> String {
    let pre = ordinal.map(|n| n.to_string()).unwrap_or_default();
    DAYS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, d)| format!("{pre}{d}")).collect::<Vec<_>>().join(",")
}

/// A recurrence pattern as (rule, deleted occurrences), given the first occurrence's start (UTC).
pub(crate) fn recurrence(blob: &[u8], start: i64) -> Option<(String, Vec<i64>)> {
    let mut b = Bytes(blob, 0);
    let (_reader, _writer) = (b.u16()?, b.u16()?);
    let frequency = b.u16()?;
    let pattern = b.u16()?;
    let _calendar = b.u16()?;
    let _first = b.u32()?;
    let period = i64::from(b.u32()?);
    let _sliding = b.u32()?;
    let (mut a, mut c) = (0, 0);
    match pattern {
        0x0001 | 0x0002 | 0x0004 | 0x000A | 0x000C => a = b.u32()?,
        0x0003 | 0x000B => {
            a = b.u32()?;
            c = b.u32()?;
        }
        _ => {}
    }
    let end_type = b.u32()?;
    let count = b.u32()?;
    let _first_dow = b.u32()?;
    let deleted_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let deleted: Vec<i64> = (0..deleted_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let modified_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let modified: Vec<i64> = (0..modified_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let first_day = i64::from(b.u32()?);
    let end_day = i64::from(b.u32()?);
    // The appointment part: when in the (local) day the occurrences start.
    let start_offset = (|| {
        b.u32()?;
        b.u32()?;
        b.u32().map(i64::from)
    })()
    .unwrap_or(0);
    let nth = |n: u32| if n >= 5 { -1 } else { i32::try_from(n).unwrap_or(1) };
    let mut rule = match (frequency, pattern) {
        (0x200A, 0x0000) => format!("FREQ=DAILY;INTERVAL={}", (period / 1440).max(1)),
        (0x200A, 0x0001) => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
        (0x200B, 0x0001) => format!("FREQ=WEEKLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, None)),
        (0x200C, 0x0002) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY={a}", period.max(1)),
        (0x200C, 0x0004) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY=-1", period.max(1)),
        (0x200C, 0x0003) => format!("FREQ=MONTHLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, Some(nth(c)))),
        (0x200D, 0x0002 | 0x0004) => format!("FREQ=YEARLY;INTERVAL={}", (period / 12).max(1)),
        (0x200D, 0x0003) => {
            let (_, month, _) = civil_from_days(start.div_euclid(86_400));
            format!("FREQ=YEARLY;INTERVAL={};BYMONTH={month};BYDAY={}", (period / 12).max(1), byday(a, Some(nth(c))))
        }
        _ => return None,
    };
    match end_type {
        0x2021 => {
            let (y, m, d) = civil_from_days((end_day - EPOCH_MINUTES).div_euclid(1440));
            rule.push_str(&format!(";UNTIL={y:04}{m:02}{d:02}T235959Z"));
        }
        0x2022 if count > 0 => rule.push_str(&format!(";COUNT={count}")),
        _ => {}
    }
    // Local day + time into it → UTC, by how far the first occurrence is from its local time.
    let local_first = (first_day - EPOCH_MINUTES + start_offset) * 60;
    let shift = start - local_first;
    let exdates = deleted.iter().filter(|d| !modified.contains(d)).map(|d| (d - EPOCH_MINUTES + start_offset) * 60 + shift).collect();
    // Sanity: a first day far from the start means the blob isn't what it says.
    if shift.abs() > 2 * 86_400 {
        return Some((rule, Vec::new()));
    }
    Some((rule, exdates))
}
