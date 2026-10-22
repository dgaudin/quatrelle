//! An account's worker: the rooms its server offers and when people and rooms are busy, for a
//! meeting being written (on the second connection, so it never waits behind a sync).

use super::*;

impl Worker {
    /// The server's room lists and rooms, answered with [`Event::Rooms`].
    pub(super) fn rooms(&mut self) {
        let r = (|| -> Result<(Vec<quatrelle_model::scheduling::RoomList>, Vec<quatrelle_model::scheduling::Room>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews_pim::scheduling::rooms(c),
                Some(Conn::Graph(c)) => crate::graph_pim::scheduling::rooms(c),
                _ => Err(NetError::No("rooms are offered by Exchange and Microsoft 365 accounts".into())),
            }
        })();
        let (lists, rooms, error) = match r {
            Ok((lists, rooms)) => (lists, rooms, None),
            Err(e) => {
                let said = e.to_string();
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.conn = None;
                }
                (Vec::new(), Vec::new(), Some(said))
            }
        };
        self.sink.emit(Event::Rooms { account: self.id(), lists, rooms, error });
    }

    /// When these people and rooms are busy between `from` and `to`, answered with
    /// [`Event::Availability`].
    pub(super) fn availability(&mut self, emails: &[String], from: i64, to: i64) {
        let r = (|| -> Result<Vec<quatrelle_model::scheduling::Availability>> {
            if emails.is_empty() {
                return Ok(Vec::new());
            }
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews_pim::scheduling::availability(c, emails, from, to),
                Some(Conn::Graph(c)) => crate::graph_pim::scheduling::availability(c, emails, from, to),
                _ => Err(NetError::No("availability comes from Exchange and Microsoft 365 accounts".into())),
            }
        })();
        let (people, error) = match r {
            Ok(people) => (people, None),
            // Refused as a whole: each one asked about hears why, so an answer never names
            // anyone it wasn't asked about (two requests can be out for one window).
            Err(e) => {
                let said = e.to_string();
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.conn = None;
                }
                let people = emails
                    .iter()
                    .map(|email| quatrelle_model::scheduling::Availability { email: email.clone(), slots: Vec::new(), error: Some(said.clone()) })
                    .collect();
                (people, Some(said))
            }
        };
        self.sink.emit(Event::Availability { account: self.id(), from, to, people, error });
    }
}
