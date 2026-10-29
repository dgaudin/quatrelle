//! Voting buttons: a message's sender offered choices ("Yes", "No"…), and the user answers with
//! one. The vote goes as a reply that names the choice, for the sender's mail client to count
//! (Exchange and Microsoft 365 accounts, which keep the choices with the message).

use super::*;

impl Session {
    /// The voting choices on a message, and the one this user sent from here (empty before).
    pub fn voting(&mut self, key: &MessageKey) -> Option<(Vec<String>, String)> {
        let m = match self.summary(key) {
            Some(m) => m.clone(),
            None => self.folder_state(&key.account, &key.folder).find(key.uid)?.clone(),
        };
        (!m.voting.is_empty()).then_some((m.voting, m.voted))
    }

    /// Send `choice` as the answer to a message's voting buttons.
    pub fn vote(&mut self, key: &MessageKey, choice: &str) -> CmdResult {
        let (choices, _) = self.voting(key).ok_or_else(|| CmdError::Disabled("this message has no voting buttons".into()))?;
        let choice = choices
            .iter()
            .find(|c| c.trim() == choice.trim())
            .ok_or_else(|| CmdError::Params(format!("`{choice}` isn't one of this message's choices")))?
            .clone();
        if self.vote_sending.as_ref().is_some_and(|(k, _)| k == key) {
            return Err(CmdError::Disabled("the vote is being sent".into()));
        }
        self.vote_sending = Some((key.clone(), choice.clone()));
        self.jobs.push(Job::Vote { key: key.clone(), choice: choice.clone() });
        Ok(serde_json::json!({ "voting": choice }))
    }

    /// The network layer's answer to [`Session::vote`].
    pub(crate) fn voted(&mut self, key: &MessageKey, choice: &str, error: Option<String>) {
        if self.vote_sending.as_ref().is_some_and(|(k, _)| k == key) {
            self.vote_sending = None;
        }
        match error {
            None => {
                for m in self.list.iter_mut().filter(|m| &m.key == key) {
                    Arc::make_mut(m).voted = choice.to_string();
                }
                let k = (key.account.clone(), key.folder.clone());
                if let Some(m) = self.cache.get_mut(&k).and_then(|f| f.find_mut(key.uid)) {
                    m.voted = choice.to_string();
                }
                self.notice_with("Your vote was sent: {choice}", &[("choice", choice)], false);
            }
            Some(e) => self.notice_with("Your vote wasn't sent: {error}", &[("error", &e)], true),
        }
    }
}
