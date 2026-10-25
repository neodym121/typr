//! Light tidying of a transcript before it is pasted: whitespace collapsed,
//! a capital letter at the start of every sentence and a punctuation mark at
//! the end. Capitals are only ever added, never removed.

/// Abbreviations whose period doesn't end a sentence (lowercase, without the
/// period). Abbreviations with an inner period (т.е., e.g.) are caught by
/// that period anyway.
const ABBREVIATIONS: &[&str] = &[
    "напр", "см", "ср", "др", "пр", "стр", "рис", "табл", "им", "ул", "пр-т", "e.g", "i.e", "vs",
    "mr", "mrs", "ms", "dr", "st", "etc",
];

/// Closing quotes and brackets that may follow a sentence's last mark.
fn is_closer(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '»' | '”' | '’' | ')' | ']' | '}')
}

/// Opening quotes and brackets that may come before a sentence's first letter.
fn is_opener(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '«' | '“' | '‘' | '(' | '[' | '{' | '-' | '—' | '–')
}

/// Whether the period that ends `word` (the text since the last space,
/// period included) closes a sentence. Not for decimals and addresses (no
/// space follows those anyway) nor for abbreviations.
fn period_ends_sentence(word: &str) -> bool {
    let body = word.trim_end_matches('.').trim_start_matches(is_opener);
    if body.contains('.') {
        // т.е. / e.g. / и т.д.: an abbreviation with inner periods
        return false;
    }
    !ABBREVIATIONS.contains(&body.to_lowercase().as_str())
}

/// The transcript as it gets pasted. Words are kept as they are, one space
/// apart; only sentence starts gain a capital and the end gains a mark.
pub fn cleanup_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 1);
    let mut starts_sentence = true;
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if starts_sentence {
            push_capitalized(&mut out, word);
        } else {
            out.push_str(word);
        }
        starts_sentence = ends_sentence(word);
    }
    if !out.is_empty() {
        close_last_sentence(&mut out);
    }
    out
}

/// Appends `word` with its first letter in upper case, past any opening
/// quotes or brackets. A word with a capital somewhere already (iPhone,
/// eBay, USA) stays as it is, and so does one that starts with a digit or
/// another symbol.
fn push_capitalized(out: &mut String, word: &str) {
    if word.chars().any(char::is_uppercase) {
        out.push_str(word);
        return;
    }
    let rest = word.trim_start_matches(is_opener);
    let mut letters = rest.chars();
    match letters.next() {
        Some(first) if first.is_alphabetic() => {
            out.push_str(&word[..word.len() - rest.len()]);
            out.extend(first.to_uppercase());
            out.push_str(letters.as_str());
        }
        _ => out.push_str(word),
    }
}

/// Whether the word after `word` starts a new sentence: `word` ends with
/// ! or ?, or with a period that isn't an abbreviation's, maybe followed by
/// closing quotes and brackets. An ellipsis doesn't count, speech usually
/// goes on after it in lower case.
fn ends_sentence(word: &str) -> bool {
    let end = word.trim_end_matches(is_closer);
    match end.chars().next_back() {
        Some('!' | '?') => true,
        Some('.') if !end.ends_with("...") => period_ends_sentence(end),
        _ => false,
    }
}

/// Makes sure the text ends with a sentence mark, before any closing quotes
/// and brackets. A trailing comma or semicolon becomes the period; with no
/// mark at all, the period goes after the closers.
fn close_last_sentence(out: &mut String) {
    let body = out.trim_end_matches(is_closer).len();
    match out[..body].chars().next_back() {
        Some('.' | '!' | '?' | '…' | ':') => {}
        Some(',' | ';') => out.replace_range(body - 1..body, "."),
        _ => out.push('.'),
    }
}

#[cfg(test)]
mod tests {
    use super::cleanup_text;

    /// Each pair is what was dictated and what gets pasted.
    fn check(cases: &[(&str, &str)]) {
        for &(dictated, pasted) in cases {
            assert_eq!(cleanup_text(dictated), pasted, "dictated: {:?}", dictated);
        }
    }

    #[test]
    fn whitespace_collapses_to_single_spaces() {
        check(&[
            ("  one more time  ", "One more time."),
            ("one    more\n\ttime", "One more time."),
            ("", ""),
            (" \n\t ", ""),
        ]);
    }

    #[test]
    fn every_sentence_starts_with_a_capital() {
        check(&[
            ("it works. ship it", "It works. Ship it."),
            ("ready? go", "Ready? Go."),
            ("stop! wait", "Stop! Wait."),
            ("привет. как дела", "Привет. Как дела."),
            ("он сказал «да.» потом ушёл", "Он сказал «да.» Потом ушёл."),
        ]);
    }

    #[test]
    fn numbers_addresses_and_abbreviations_end_nothing() {
        check(&[
            ("версия 3.14 вышла", "Версия 3.14 вышла."),
            ("open example.com now", "Open example.com now."),
            ("это т.е. вот так", "Это т.е. вот так."),
            ("яблоки, груши и т.д. и т.п.", "Яблоки, груши и т.д. и т.п."),
            ("см. рисунок ниже", "См. рисунок ниже."),
            ("use tools e.g. hammers", "Use tools e.g. hammers."),
        ]);
    }

    #[test]
    fn an_ellipsis_goes_on_in_lowercase() {
        check(&[("ну... ладно", "Ну... ладно."), ("ну… ладно", "Ну… ладно.")]);
    }

    #[test]
    fn numbers_quotes_and_capitals_inside_words() {
        check(&[
            ("5 apples. «ok» then", "5 apples. «Ok» then."),
            ("iPhone is here", "iPhone is here."),
        ]);
    }

    #[test]
    fn the_text_ends_with_one_mark() {
        check(&[
            ("that is all", "That is all."),
            ("that is all.", "That is all."),
            ("that is all!", "That is all!"),
            ("is that all?", "Is that all?"),
            ("и так далее…", "И так далее…"),
            ("he said \"stop!\"", "He said \"stop!\""),
            ("(как-то так)", "(Как-то так)."),
            ("список:", "Список:"),
            ("that is all,", "That is all."),
            ("That is all.", "That is all."),
        ]);
    }
}
