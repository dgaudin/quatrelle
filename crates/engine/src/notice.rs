//! Status-bar messages. The engine writes them in English; one with values in it keeps its
//! wording and its values apart, so the interface can say it in its own language.

use crate::Session;

/// The most notices kept: older ones go first.
const NOTICE_MAX: usize = 50;

/// A status-bar message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// In English with its values filled in: what the CLI, agents and tests read.
    pub text: String,
    /// The English wording `text` was filled from, `{name}` placeholders kept: what the
    /// interface translates. Empty for a text given whole (a fixed wording, or an error said in
    /// full), which is looked up as it is.
    pub template: &'static str,
    /// The values of `template`'s placeholders. One named `error` is an error's text, and `when`
    /// and `until` a date as `dates` writes it: the interface translates their wording as far as
    /// it is Quatrelle's. Any other (a name, a number, a subject) is put in as it is.
    pub values: Vec<(&'static str, String)>,
    pub at: i64,
    pub error: bool,
}

/// `template` with each `{name}` filled in from `values`, in one pass: a value is never read as
/// a template, and a name without a value is left as it is.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, tail)) = after.split_once('}') else {
            out.push('{');
            out.push_str(after);
            return out;
        };
        match values.iter().find(|(k, _)| *k == name) {
            Some((_, v)) => out.push_str(v),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

impl Session {
    /// A message for the status bar given whole: a fixed wording, or an error said in full.
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        let at = self.clock.now;
        self.push_notice(Notice { text: text.into(), template: "", values: Vec::new(), at, error });
    }

    /// A message for the status bar from an English wording with `{name}` placeholders and
    /// their values (see [`Notice::values`] for the names the interface reads as more than text).
    pub fn notice_with(&mut self, template: &'static str, values: &[(&'static str, &str)], error: bool) {
        let at = self.clock.now;
        let text = fill(template, values);
        let values = values.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
        self.push_notice(Notice { text, template, values, at, error });
    }

    fn push_notice(&mut self, n: Notice) {
        self.notices.push(n);
        if self.notices.len() > NOTICE_MAX {
            self.notices.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_filled_in_one_pass() {
        assert_eq!(fill("Saved {n} in {folder}", &[("n", "3"), ("folder", "{n}")]), "Saved 3 in {n}", "values aren't templates");
        assert_eq!(fill("{missing} ok", &[]), "{missing} ok");
        assert_eq!(fill("open {brace", &[("brace", "x")]), "open {brace");
        assert_eq!(fill("", &[]), "");
    }

    #[test]
    fn a_notice_keeps_its_wording_and_its_values_apart() {
        let mut s = Session::open(None, quatrelle_model::dates::Clock::default());
        s.notice_with("the server's rules weren't changed: {error}", &[("error", "the server refused: no")], true);
        let n = s.notices.last().cloned();
        assert_eq!(n.as_ref().map(|n| n.text.as_str()), Some("the server's rules weren't changed: the server refused: no"));
        assert_eq!(n.as_ref().map(|n| n.template), Some("the server's rules weren't changed: {error}"));
        assert_eq!(n.map(|n| n.values), Some(vec![("error", "the server refused: no".to_string())]));
        s.notice("Draft saved", false);
        assert_eq!(s.notices.last().map(|n| (n.template, n.values.len())), Some(("", 0)));
        for _ in 0..NOTICE_MAX + 5 {
            s.notice("x", false);
        }
        assert_eq!(s.notices.len(), NOTICE_MAX);
    }
}
