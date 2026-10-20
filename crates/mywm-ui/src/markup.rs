//! The markup of notification bodies (the spec's `<b>`, `<i>`, `<u>`, `<a href>`, `<img>`), read as a QML `Text` with
//! `textFormat: Text.StyledText` reads it: bold, italic and underlined runs, line breaks for `<br>` and new lines,
//! the usual entities; links show as underlined text, pictures and unknown tags not at all.

/// A run of text in one style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// The runs of `markup`, neighbours of the same style joined.
pub fn parse(markup: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic, mut underline) = (0u32, 0u32, 0u32);
    let mut push = |text: &str, bold: u32, italic: u32, underline: u32| {
        if text.is_empty() {
            return;
        }
        let style = (bold > 0, italic > 0, underline > 0);
        match spans.last_mut() {
            Some(last) if (last.bold, last.italic, last.underline) == style => last.text.push_str(text),
            _ => spans.push(Span { text: text.to_owned(), bold: style.0, italic: style.1, underline: style.2 }),
        }
    };
    let mut rest = markup;
    while !rest.is_empty() {
        let Some(open) = rest.find(['<', '&']) else {
            push(rest, bold, italic, underline);
            break;
        };
        push(&rest[..open], bold, italic, underline);
        rest = &rest[open..];
        if rest.starts_with('&') {
            match rest.find(';').filter(|end| *end <= 10).and_then(|end| Some((entity(&rest[1..end])?, end))) {
                Some((character, end)) => {
                    push(character.encode_utf8(&mut [0; 4]), bold, italic, underline);
                    rest = &rest[end + 1..];
                }
                None => {
                    push("&", bold, italic, underline);
                    rest = &rest[1..];
                }
            }
            continue;
        }
        // A `<` that opens no tag (not followed by a name or `/`, or never closed) is text.
        let opens = rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/');
        let Some(close) = rest.find('>').filter(|_| opens) else {
            push("<", bold, italic, underline);
            rest = &rest[1..];
            continue;
        };
        let tag = rest[1..close].trim();
        rest = &rest[close + 1..];
        let (closing, name) = match tag.strip_prefix('/') {
            Some(name) => (true, name.trim()),
            None => (false, tag.trim_end_matches('/').trim()),
        };
        let name = name.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        let change = |counter: &mut u32| if closing { *counter = counter.saturating_sub(1) } else { *counter += 1 };
        match name.as_str() {
            "b" | "strong" => change(&mut bold),
            "i" | "em" => change(&mut italic),
            "u" | "a" => change(&mut underline),
            "br" => push("\n", bold, italic, underline),
            "p" if closing => push("\n", bold, italic, underline),
            _ => {}
        }
    }
    spans
}

/// The character an entity (`amp`, `#39`, `#x27`) stands for.
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let number = name.strip_prefix('#')?;
            let value = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(value)
        }
    }
}

/// The plain text of `spans`.
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, bold: bool, italic: bool, underline: bool) -> Span {
        Span { text: text.into(), bold, italic, underline }
    }

    #[test]
    fn styles_nest_and_join() {
        let spans = parse("plain <b>bold <i>both</i></b> <u>under</u><a href=\"x\">link</a>");
        assert_eq!(
            spans,
            vec![
                span("plain ", false, false, false),
                span("bold ", true, false, false),
                span("both", true, true, false),
                span(" ", false, false, false),
                span("underlink", false, false, true),
            ]
        );
    }

    #[test]
    fn entities_breaks_and_stray_characters() {
        assert_eq!(plain(&parse("a &amp; b &lt;c&gt; &#39;d&#x27; &bogus; x")), "a & b <c> 'd' &bogus; x");
        assert_eq!(plain(&parse("one<br/>two<br>three\nfour")), "one\ntwo\nthree\nfour");
        assert_eq!(plain(&parse("1 < 2 and <img src=\"x\"/>done")), "1 < 2 and done");
        assert_eq!(plain(&parse("unclosed <b")), "unclosed <b");
        assert_eq!(parse(""), vec![]);
    }
}
