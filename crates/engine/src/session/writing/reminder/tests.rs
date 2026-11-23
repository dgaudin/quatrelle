// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reminder's tests: what each language the interface speaks says when something goes with
//! a message, and the boundary each script needs. Made-up text only.

use super::*;

/// Whether what was written (its subject and body) says something goes with the message.
fn says(subject: &str, body: &str) -> bool {
    mentions_attachment(subject, body)
}

/// A script a folded phrase can be written in, grouped the way [`Edge`] groups them (Han and
/// kana together: Japanese mixes them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Arabic,
    Hebrew,
    HanKana,
    Hangul,
}

/// The script of one letter of a phrase; `None` for the separators a phrase may hold.
fn script(c: char) -> Option<Script> {
    match c {
        'a'..='z' => Some(Script::Latin),
        '\u{400}'..='\u{4FF}' => Some(Script::Cyrillic),
        '\u{600}'..='\u{6FF}' => Some(Script::Arabic),
        '\u{590}'..='\u{5FF}' => Some(Script::Hebrew),
        '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' => Some(Script::HanKana),
        '\u{1100}'..='\u{11FF}' | '\u{AC00}'..='\u{D7A3}' => Some(Script::Hangul),
        _ => None,
    }
}

/// Every phrase of the table is written in one script only: a phrase mixing two of them (an
/// Arabic letter inside a Hebrew word, say) could never match what people write.
#[test]
fn every_phrase_is_written_in_a_single_script() {
    for (phrase, _) in ATTACHMENT_PHRASES {
        let mut written = None;
        for c in phrase.chars() {
            match script(c) {
                Some(s) => {
                    assert!(written.is_none_or(|w| w == s), "{phrase:?}: {c:?} is {s:?}, the phrase began as {written:?}");
                    written = Some(s);
                }
                None => assert!(matches!(c, '-' | ' '), "{phrase:?}: {c:?} is no letter and no separator"),
            }
        }
    }
}

/// The Latin-script languages read exactly as before: a whole word, and neither a longer word
/// containing it ("unattached") nor a link.
#[test]
fn the_latin_script_languages_read_as_before() {
    assert!(says("Budget", "Please find attached the numbers."));
    assert!(says("Plan", "Ci joint le budget."));
    assert!(says("Plan", "Anbei die Zahlen."));
    assert!(says("Plan", "Zie bijgevoegd."));
    assert!(!says("Plan", "The plan stays unattached to the timeline."));
    assert!(!says("Plan", "See https://files.contoso.example/plan.pdf"));
}

/// Russian: folded lower case, ё read as е. A noun's cases and a verb stem's endings run on, so
/// the boundary is before the phrase only; "приложение" (also "application") says nothing.
#[test]
fn russian() {
    assert!(says("Коммерческое предложение во вложении", "Направляю его отдельным письмом."));
    assert!(says("", "См. во вложении договор."));
    assert!(says("", "Прилагаю договор к письму."));
    assert!(says("", "Счёт прилагается."));
    assert!(says("", "Акты прилагаем к ответу."));
    assert!(says("", "Прикрепляю счёт."));
    assert!(says("", "Прикреплённый файл — во вложении."));
    assert!(says("", "Вчера приложил файлы."));
    // The trap: приложение also means "application"; a longer word containing a phrase
    // ("неприкреплённый", not attached) doesn't count; a link says nothing.
    assert!(!says("", "Скачайте наше приложение."));
    assert!(!says("", "Это приложение удобное."));
    assert!(!says("", "Файл неприкреплённый, см. ссылку."));
    assert!(!says("", "https://contoso.example/app"));
}

/// Arabic: the one-letter prefixes glued in front of a word (و ف ب ل ك, and the article's ا of
/// ال) may stand before the phrase, bounded themselves; a verb takes a glued pronoun after it.
#[test]
fn arabic() {
    assert!(says("إرسال المرفقات", "الملف مرفق مع الرسالة."));
    assert!(says("", "تجدون المرفقات في الرسالة."));
    assert!(says("", "وبالمرفقات تجدون كل شيء."));
    assert!(says("", "نرفق لكم الملفات."));
    assert!(says("", "أرفق لكم الملف."));
    assert!(says("", "الوثيقة مرفقة."));
    assert!(says("", "أرسلت المرفقه."));
    assert!(says("", "طيه الملف."));
    // "File" alone isn't a promise of anything; a link.
    assert!(!says("", "انظر الملف."));
    assert!(!says("", "https://files.contoso.example/report.pdf"));
}

/// Hebrew: the one-letter prefixes (ו ב ש ה כ ל מ) may stand before the phrase; a verb takes its
/// ending or a glued pronoun after it. "צירוף מקרים" (a coincidence) says nothing.
#[test]
fn hebrew() {
    // The singular מצורף, in a sentence and in the subject, with the glued prefixes ה and ב.
    assert!(says("הקובץ מצורף", ""));
    assert!(says("", "הקובץ מצורף. תודה"));
    assert!(says("", "בקובץ המצורף."));
    // The plural, the feminine, the verbs.
    assert!(says("", "קבצים מצורפים."));
    assert!(says("", "התעודה מצורפת."));
    assert!(says("", "מצירף את הקובץ."));
    assert!(says("", "צירפתי את הקובץ."));
    // A coincidence, a file alone in a folder, a Latin letter glued in front, a link.
    assert!(!says("", "צירוף מקרים."));
    assert!(!says("", "הקובץ בתיקייה."));
    assert!(!says("", "fileמצורף"));
    assert!(!says("", "https://files.contoso.example/report.pdf"));
}

/// Japanese: written without spaces, so no boundary at all — the phrase lights inside a longer
/// run of text (添付ファイル, ファイルを添付しました). Full-width ASCII folds to ASCII.
#[test]
fn japanese() {
    assert!(says("資料のご確認（添付）", "よろしくお願いします。"));
    assert!(says("", "添付ファイルをご確認ください。"));
    assert!(says("", "ファイルを添付しました。"));
    assert!(says("", "別添のとおりです。"));
    assert!(says("", "書類を同封します。"));
    assert!(says("", "ＦＩＬＥを添付")); // full-width letters fold to ASCII
    // No phrase, and a link.
    assert!(!says("", "契約書は後ほどお送りします。"));
    assert!(!says("", "https://files.contoso.example/契約書.pdf"));
}

/// Korean: a boundary before only, because a particle is glued behind the word (첨부파일을,
/// 첨부합니다). "미첨부" (not attached) has a letter before and doesn't count.
#[test]
fn korean() {
    assert!(says("첨부 파일 안내", "견적서를 보내드립니다."));
    assert!(says("", "첨부파일을 보내드립니다."));
    assert!(says("", "첨부합니다."));
    assert!(says("", "동봉합니다."));
    // Not attached, and a link.
    assert!(!says("", "미첨부 상태입니다."));
    assert!(!says("", "https://files.contoso.example/report.pdf"));
}

/// Chinese, simplified and traditional: written without spaces, no boundary. 附 alone is too
/// short (附录, an appendix; 附言, a PS), so it isn't a phrase.
#[test]
fn chinese_simplified_and_traditional() {
    assert!(says("报价单（请查收附件）", "合同已签字。"));
    assert!(says("", "请查收附件。"));
    assert!(says("", "文件已附上。"));
    assert!(says("", "随附文件如下。"));
    assert!(says("", "随函附有报价单。"));
    assert!(says("", "請查閱附件。"));
    assert!(says("", "檔案已附上。"));
    assert!(says("", "隨附檔案如下。"));
    assert!(says("", "隨函附有報價單。"));
    // An appendix, a PS, a link.
    assert!(!says("", "详见附录。"));
    assert!(!says("", "附言：谢谢。"));
    assert!(!says("", "https://files.contoso.example/合同.pdf"));
}

/// A reply's original quoted under it, in each of the seven languages: the header words
/// ([`mime::QuoteWords`], filled from the interface's catalogs) cut it out of what the user
/// wrote, so a phrase only in the original doesn't hold the message back; the user's own words
/// above the quote still count.
#[test]
fn a_quoted_original_in_each_language_is_not_the_users_words() {
    // The header words as the catalogs of the seven languages write them (From, Sent, Subject),
    // the original's own sentence, and the user's own sentence above the quote.
    for (from, sent, subject, original, own) in [
        ("От:", "Отправлено:", "Тема:", "Во вложении отправляю договор.", "Договор прилагается."),
        ("差出人:", "送信日時:", "件名:", "契約書を添付しました。", "資料を添付します。"),
        ("보낸 사람:", "보낸 날짜:", "제목:", "첨부파일을 보냅니다.", "첨부합니다."),
        ("发件人:", "发送时间:", "主题:", "附件已附上。", "请查收附件。"),
        ("寄件者:", "寄件日期:", "主旨:", "附件已附上。", "請查收附件。"),
        ("من:", "تاريخ الإرسال:", "الموضوع:", "الملف مرفق بالرسالة.", "المرفق مع الرسالة."),
        ("מאת:", "נשלח:", "נושא:", "בקובץ המצורף.", "הקובץ מצורף, תודה"),
    ] {
        let words = mime::QuoteWords { from: from.into(), sent: sent.into(), subject: subject.into(), ..Default::default() };
        let quote = format!("{from} Ana Lima <ana@soundlab.example>\n{sent} Monday, 24 November 2026 10:00\n{subject} Plans\n\n{original}\n");
        // Only the original says it: nothing to ask.
        let c = Compose { subject: "RE: Plans".into(), body: format!("Ok.\n\n{quote}"), ..Default::default() };
        let (own_text, subject) = own_words(&c, &words);
        assert!(!mentions_attachment(subject, &own_text), "{from}: the original's words are cut out: {own_text:?}");
        // The user's own words above the quote: asked.
        let c = Compose { subject: "RE: Plans".into(), body: format!("{own}\n\n{quote}"), ..Default::default() };
        let (own_text, subject) = own_words(&c, &words);
        assert!(mentions_attachment(subject, &own_text), "{from}: the user's own words count: {own_text:?}");
    }
}
