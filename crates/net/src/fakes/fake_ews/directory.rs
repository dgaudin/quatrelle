//! The fake EWS server's directory: names resolved (`ResolveNames`) from the other mailboxes, the
//! Global Address List and, when the account's contacts are searched too, its contact groups,
//! each with its `MailboxType`; and a list's members (`ExpandDL`).

use super::*;

fn ok(inner: &str) -> String {
    format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>")
}

impl Data {
    /// A mailbox's `MailboxType`, as set for its address (`Mailbox` otherwise).
    fn mailbox_type(&self, address: &str) -> &str {
        self.mailbox_types.iter().find(|(a, _)| a.eq_ignore_ascii_case(address)).map_or("Mailbox", |(_, t)| t.as_str())
    }

    /// A person or list as a `Mailbox` element.
    fn mailbox_xml(&self, name: &str, address: &str) -> String {
        format!(
            "<t:Mailbox><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType><t:MailboxType>{}</t:MailboxType></t:Mailbox>",
            esc(name),
            esc(address),
            esc(self.mailbox_type(address))
        )
    }

    /// `ResolveNames`: an exact address or Exchange address (another mailbox, or a person the
    /// directory knows), else the people whose name or address begins so and (searching the
    /// contacts too) the contact groups whose name begins so.
    pub(super) fn resolve_names(&mut self, op: &El) -> (String, Vec<String>) {
        let name = "ResolveNamesResponse".to_string();
        let scope = op.attr("SearchScope").unwrap_or("ActiveDirectory").to_string();
        if self.scopes.len() < 1000 {
            self.scopes.push(scope.clone());
        }
        let entry = op.child("UnresolvedEntry").map(|e| e.text.clone()).unwrap_or_default();
        // `SMTP:` asks for that exact address.
        let exact = entry.starts_with("SMTP:");
        let want = entry.strip_prefix("SMTP:").map_or(entry.clone(), str::to_string);
        // Another mailbox the server names by its Exchange address (its LegacyDN), or a person the
        // directory knows (X.500 name → address).
        let found = self
            .shared
            .iter()
            .find(|b| legacy_dn(&b.address).eq_ignore_ascii_case(&want) || b.address.eq_ignore_ascii_case(&want))
            .map(|b| (b.name.clone(), b.address.clone()))
            .or_else(|| self.directory.iter().find(|(dn, _)| dn.eq_ignore_ascii_case(&want)).map(|(_, email)| (email.clone(), email.clone())));
        let mut set: Vec<String> = Vec::new();
        if let Some((n, e)) = &found {
            set.push(self.mailbox_xml(n, e));
        } else {
            // A name or an address begun (Exchange's ambiguous name resolution).
            let w = want.to_lowercase();
            let begun = |name: &str, email: &str| {
                !w.is_empty() && (email.to_lowercase().starts_with(&w) || name.to_lowercase().split(' ').any(|p| p.starts_with(&w)))
            };
            for (n, e) in self.people.iter().filter(|(n, e)| begun(n, e)) {
                set.push(self.mailbox_xml(n, e));
            }
            // The account's contact groups are in its Contacts folder, not in the directory.
            if scope != "ActiveDirectory" && !exact {
                for (n, id) in self.contact_groups.iter().filter(|(n, _)| !w.is_empty() && n.to_lowercase().starts_with(&w)) {
                    set.push(format!(
                        "<t:Mailbox><t:Name>{}</t:Name><t:ItemId Id=\"{}\" ChangeKey=\"EQ\"/><t:RoutingType>MAPIPDL</t:RoutingType>\
                         <t:MailboxType>PrivateDL</t:MailboxType></t:Mailbox>",
                        esc(n),
                        esc(id)
                    ));
                }
            }
        }
        let resolutions: String = set.iter().map(|m| format!("<t:Resolution>{m}</t:Resolution>")).collect();
        let answer = match set.len() {
            0 => "<m:R ResponseClass=\"Error\"><m:MessageText>No results were found.</m:MessageText>\
                  <m:ResponseCode>ErrorNameResolutionNoResults</m:ResponseCode></m:R>"
                .to_string(),
            1 => ok(&format!("<m:ResolutionSet TotalItemsInView=\"1\" IncludesLastItemInRange=\"true\">{resolutions}</m:ResolutionSet>")),
            n => format!(
                "<m:R ResponseClass=\"Warning\"><m:ResponseCode>ErrorNameResolutionMultipleResults</m:ResponseCode>\
                 <m:ResolutionSet TotalItemsInView=\"{n}\" IncludesLastItemInRange=\"true\">{resolutions}</m:ResolutionSet></m:R>"
            ),
        };
        (name, vec![answer])
    }

    /// `ExpandDL`: the members of the list named by its address, or of the contact group named by
    /// its item id; a list nobody has, none.
    pub(super) fn expand_dl(&mut self, op: &El) -> (String, Vec<String>) {
        let mailbox = op.child("Mailbox");
        let id = mailbox.and_then(|m| m.child("ItemId")).and_then(|i| i.attr("Id")).map(str::to_string);
        let address = mailbox.and_then(|m| m.text_of("EmailAddress")).unwrap_or("").trim().to_string();
        let members = self
            .list_members
            .iter()
            .find(|(k, _)| match &id {
                Some(id) => k == id,
                None => !address.is_empty() && k.eq_ignore_ascii_case(&address),
            })
            .map(|(_, m)| m.as_slice())
            .unwrap_or_default();
        let list: String = members.iter().map(|(n, e)| self.mailbox_xml(n, e)).collect();
        let n = members.len();
        (
            "ExpandDLResponse".into(),
            vec![ok(&format!("<m:DLExpansion TotalItemsInView=\"{n}\" IncludesLastItemInRange=\"true\">{list}</m:DLExpansion>"))],
        )
    }
}
