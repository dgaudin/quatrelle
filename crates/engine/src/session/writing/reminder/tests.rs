// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reminder's tests: what each language the interface speaks says when something goes with
//! a message, and the boundary each script needs. Made-up text only.

use super::*;

/// Whether what was written (its subject and body) says something goes with the message.
fn says(subject: &str, body: &str) -> bool {
    mentions_attachment(subject, body)
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

/// Russian: folded lower case, ё read as е. A verb stem carries its ending, so the boundary is
/// before the phrase only; "приложение" (also "application") says nothing.
#[test]
fn russian() {
    // The phrase alone, in a sentence, capitalised, with ё, and in the subject.
    assert!(says("Вложение", ""));
    assert!(says("", "См. во вложении."));
    assert!(says("", "Вложение направляю отдельным письмом."));
    assert!(says("", "Прилагаю договор."));
    assert!(says("", "Прикрепляю счёт."));
    assert!(says("", "Прикреплённый файл — ниже."));
    assert!(says("", "Вложенный файл во вложении."));
    // The trap: приложение also means "application"; a longer word containing a phrase
    // ("неприкреплённый", not attached) doesn't count; a link says nothing.
    assert!(!says("", "Скачайте наше приложение."));
    assert!(!says("", "Файл неприкреплённый, см. ссылку."));
    assert!(!says("", "https://contoso.example/app"));
}

/// Arabic: the one-letter prefixes glued in front of a word (و ف ب ل ك, and the article's ا of
/// ال) may stand before the phrase, bounded themselves; a verb takes a glued pronoun after it.
#[test]
fn arabic() {
    assert!(says("المرفق", ""));
    assert!(says("", "المرفق أدناه."));
    assert!(says("", "الملف مرفق."));
    assert!(says("", "وبالمرفقات.")); // و + ب + ال + مرفقات: four glued prefixes
    assert!(says("", "بالمرفق أدناه."));
    assert!(says("", "أرفق لكم الملف."));
    assert!(says("", "أرفقه.")); // with the glued pronoun ه
    // "File" alone isn't a promise of anything; a letter glued in front that isn't a prefix
    // ("fileمرفق"); a link.
    assert!(!says("", "انظر الملف."));
    assert!(!says("", "fileمرفق"));
    assert!(!says("", "https://files.contoso.example/report.pdf"));
}

/// Hebrew: the one-letter prefixes (ו ב ש ה כ ל מ) may stand before the phrase; a verb takes its
/// ending or a glued pronoun after it. "צירוף מקרים" (a coincidence) says nothing.
#[test]
fn hebrew() {
    assert!(says("קובץ מצترف", ""));
    assert!(says("", "הקובץ מצترف."));
    assert!(says("", "קבצים מצורפים."));
    assert!(says("", "המסמך מצורפת."));
    assert!(says("", "מצירף את הקובץ."));
    assert!(says("", "צירפתי את הקובץ."));
    // A coincidence, a file alone, a letter glued in front that isn't a prefix, a link.
    assert!(!says("", "צירוף מקרים."));
    assert!(!says("", "siehe קובץ allein."));
    assert!(!says("", "fileמצترف"));
    assert!(!says("", "https://files.contoso.example/report.pdf"));
}

/// Japanese: written without spaces, so no boundary at all — the phrase lights inside a longer
/// run of text (添付ファイル, ファイルを添付しました). Full-width ASCII folds to ASCII.
#[test]
fn japanese() {
    assert!(says("資料添付", ""));
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
    assert!(says("파일 첨부", ""));
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
    assert!(says("附件：报价单", ""));
    assert!(says("", "请查收附件。"));
    assert!(says("", "文件已附上。"));
    assert!(says("", "請查閱附件。"));
    assert!(says("", "檔案已附上。"));
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
    // with a phrase in the original's body.
    for (from, sent, subject, phrase) in [
        ("От:", "Отправлено:", "Тема:", "во вложении"),
        ("差出人:", "送信日時:", "件名:", "添付"),
        ("보낸 사람:", "보낸 날짜:", "제목:", "첨부"),
        ("发件人:", "发送时间:", "主题:", "附件"),
        ("寄件者:", "寄件日期:", "主旨:", "附件"),
        ("من:", "تاريخ الإرسال:", "الموضوع:", "مرفق"),
        ("מאת:", "נשלח:", "נושא:", "מצترف"),
    ] {
        let words = mime::QuoteWords { from: from.into(), sent: sent.into(), subject: subject.into(), ..Default::default() };
        let quote = format!(
            "{from} Ana Lima <ana@soundlab.example>\n{sent} Monday, 24 November 2026 10:00\n{subject} Plans\n\nHere is the file, {phrase}.\n"
        );
        // Only the original says it: nothing to ask.
        let c = Compose { subject: "RE: Plans".into(), body: format!("Thanks, got it.\n\n{quote}"), ..Default::default() };
        let (own, subject) = own_words(&c, &words);
        assert!(!mentions_attachment(subject, &own), "{from}: the original's words are cut out: {own:?}");
        // The user's own words above the quote: asked.
        let c = Compose { subject: "RE: Plans".into(), body: format!("Ok, {phrase}.\n\n{quote}"), ..Default::default() };
        let (own, subject) = own_words(&c, &words);
        assert!(mentions_attachment(subject, &own), "{from}: the user's own words count: {own:?}");
    }
}
