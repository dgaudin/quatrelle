//! EWS: the master category list, as Outlook keeps it in the mailbox — the categories it offers,
//! by name and colour (MS-OXOCFG: the `CategoryList` configuration of the Calendar folder).

use super::*;

/// Most categories taken from the list.
const MAX: usize = 500;

/// The categories Outlook made in this mailbox, each with its colour in Quatrelle's palette (none
/// when Outlook gave it none). A mailbox Outlook never opened has no list: none.
pub fn master_categories(c: &Client) -> Result<Vec<(String, Option<u8>)>> {
    let m = match c.call_one(
        "<m:GetUserConfiguration><m:UserConfigurationName Name=\"CategoryList\"><t:DistinguishedFolderId Id=\"calendar\"/>\
         </m:UserConfigurationName><m:UserConfigurationProperties>XmlData</m:UserConfigurationProperties></m:GetUserConfiguration>",
    ) {
        Ok(m) => m,
        // None kept (`ErrorItemNotFound`), or refused: no list to go by.
        Err(NetError::No(e)) => {
            log::info!("no category list: {e}");
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };
    let data = m.find("XmlData").map(|d| d.text.trim().to_string()).unwrap_or_default();
    let xml = crate::ews::unb64(&data).ok_or_else(|| NetError::Protocol("the category list isn't base64".into()))?;
    Ok(categories_in(&String::from_utf8_lossy(&xml)))
}

/// The categories of a `CategoryList` document: `<category name="…" color="7" …/>` each.
pub(super) fn categories_in(xml: &str) -> Vec<(String, Option<u8>)> {
    let doc = crate::ews::parse(xml.trim_start_matches('\u{feff}'));
    let mut found = Vec::new();
    doc.all("category", &mut found);
    found
        .into_iter()
        .filter_map(|c| {
            let name = c.attr("name")?.trim().to_string();
            let color = c.attr("color").and_then(|v| v.trim().parse::<i32>().ok()).and_then(quatrelle_model::organize::outlook_color);
            (!name.is_empty()).then_some((name, color))
        })
        .take(MAX)
        .collect()
}
