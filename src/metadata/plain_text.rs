//! A provider's text as plain text: what is stored and sent is never markup.

/// `text` without its HTML tags — a `<br>` or `<p>` becomes a line break — with HTML entities
/// decoded, trimmed, each line's end trimmed and at most one empty line in a row; `None` when
/// nothing is left.
pub fn plain_text(text: &str) -> Option<String> {
    let decoded = decode_entities(&without_tags(text));
    let mut lines: Vec<&str> = Vec::new();
    for line in decoded.lines().map(str::trim_end) {
        let follows_empty = lines.last().is_none_or(|last| last.is_empty());
        if !(line.is_empty() && follows_empty) {
            lines.push(line);
        }
    }
    let joined = lines.join("\n");
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `text` with every tag left out. A `<` that opens no tag — `a < b`, or one never closed — stays.
fn without_tags(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        plain.push_str(&rest[..start]);
        let tag = &rest[start + 1..];
        let opens_tag = tag
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '/' || first == '!');
        let Some(end) = tag.find('>').filter(|_| opens_tag) else {
            plain.push('<');
            rest = tag;
            continue;
        };
        rest = &tag[end + 1..];
        let name: String = tag[..end]
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_lowercase();
        // Providers write a break as `<br>` followed by the line break itself; only one is kept.
        let breaks_line = (name == "br" || name == "p") && !rest.starts_with(['\n', '\r']);
        if breaks_line {
            plain.push('\n');
        }
    }
    plain.push_str(rest);
    plain
}

/// `text` with `&name;`, `&#NNN;` and `&#xHH;` replaced by their characters. An entity this
/// doesn't know, or an `&` that starts none, stays as it is.
fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        // The longest entity decoded here, `&#x10FFFF;`, has 8 characters between `&` and `;`.
        let character = after
            .find(';')
            .filter(|&end| end <= 8)
            .and_then(|end| Some((entity(&after[..end])?, end)));
        match character {
            Some((character, end)) => {
                decoded.push(character);
                rest = &after[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = after;
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// The character an entity's name stands for: a number, or one of the names provider text uses.
fn entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code).filter(|&character| character != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        plain_text(text).expect("something is left")
    }

    #[test]
    fn tags_are_left_out_and_a_break_becomes_one_line_break() {
        assert_eq!(plain("A <i>dark</i> <b>tale</b>."), "A dark tale.");
        assert_eq!(
            plain("One<br>two<br/>three<BR />four"),
            "One\ntwo\nthree\nfour"
        );
        assert_eq!(plain("One<br>\ntwo<br>\r\nthree"), "One\ntwo\nthree");
        assert_eq!(plain("<p>One</p><p>Two</p>"), "One\n\nTwo");
        assert_eq!(
            plain(r#"See <a href="https://anilist.co/manga/1/">Chapter 83</a>."#),
            "See Chapter 83."
        );
        assert_eq!(plain("Before<!-- a comment -->after"), "Beforeafter");
    }

    #[test]
    fn a_less_than_sign_that_opens_no_tag_stays() {
        assert_eq!(plain("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
        assert_eq!(plain("<3 forever"), "<3 forever");
        assert_eq!(plain("Never closed <b"), "Never closed <b");
    }

    #[test]
    fn entities_are_decoded_once_and_unknown_ones_stay() {
        assert_eq!(
            plain("Tom &amp; Jerry &mdash; &quot;Hi&quot; &#39;x&#039; &#x2014; &hellip;"),
            "Tom & Jerry — \"Hi\" 'x' — …"
        );
        assert_eq!(
            plain("&lt;b&gt;stays text&lt;/b&gt;"),
            "<b>stays text</b>",
            "an escaped tag is text, not markup"
        );
        assert_eq!(plain("&amp;lt;"), "&lt;", "decoded once");
        assert_eq!(plain("Tom & Jerry; R&D"), "Tom & Jerry; R&D");
        assert_eq!(
            plain("&unknown; &#0; &#xFFFFFFFF; &#12a;"),
            "&unknown; &#0; &#xFFFFFFFF; &#12a;"
        );
        assert_eq!(plain("&averyverylongname;"), "&averyverylongname;");
    }

    #[test]
    fn line_ends_are_trimmed_and_empty_lines_never_follow_each_other() {
        assert_eq!(
            plain("\n\n  First  \n\n\n\nSecond<br><br><br>Third\n\n"),
            "First\n\nSecond\n\nThird"
        );
    }

    #[test]
    fn text_with_nothing_left_is_none() {
        for empty in ["", "   ", "<br><br>", "<i> </i>\n\n"] {
            assert_eq!(plain_text(empty), None, "{empty:?}");
        }
    }

    #[test]
    fn an_anilist_description_reads_as_plain_text() {
        let description = "His name is Guts, the Black Swordsman.<br>\n<br>\n(Source: Dark Horse)<br>\n<br>\n<i>Notes:<br>\n- <a href=\"https://anilist.co/manga/95821/\">Chapter 83</a> was omitted from Volume 13 due to the author’s request.</i>";

        assert_eq!(
            plain(description),
            "His name is Guts, the Black Swordsman.\n\n(Source: Dark Horse)\n\nNotes:\n- Chapter 83 was omitted from Volume 13 due to the author’s request."
        );
    }
}
