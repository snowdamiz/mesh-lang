//! String value semantics shared by compilation and static test metadata.

use crate::SyntaxKind;

/// Process a STRING_CONTENT segment from a triple-quoted heredoc.
///
/// - `is_first`: strip the leading newline (the one after the opening `"""`)
/// - `trim_level`: strip this many leading spaces from each line
/// - For the last segment, the last line contains only the closing indent — it is dropped.
///   Detection: if the last line is all-whitespace (pure spaces/tabs), it is the closing
///   indent line and must be stripped.
pub fn apply_heredoc_content(
    text: String,
    is_first: bool,
    is_last: bool,
    trim_level: usize,
) -> String {
    // Strip leading newline from first segment
    let s: String = if is_first {
        text.strip_prefix("\r\n")
            .or_else(|| text.strip_prefix('\n'))
            .map_or_else(|| text.clone(), str::to_string)
    } else {
        text
    };

    // Split into lines to process each one
    let mut lines: Vec<&str> = s.split('\n').collect();

    // The closing segment ends with the indent line before `"""`: drop it.
    // An earlier segment's whitespace-only last line is the indent before an
    // interpolation that starts a line; it is dedented like any other line.
    if is_last
        && lines
            .last()
            .map(|l| l.chars().all(|c| c == ' ' || c == '\t'))
            .unwrap_or(false)
    {
        lines.pop();
    }

    let stripped_lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 && !is_first {
                // Text right after an interpolation continues that line.
                return line.to_string();
            }
            // Strip the common indentation, or as much of it as the line has.
            let leading_ws: usize = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            line[leading_ws.min(trim_level)..].to_string()
        })
        .collect();

    stripped_lines.join("\n")
}

/// Process escape sequences in a raw string token, converting `\"` → `"`,
/// `\\` → `\`, `\n` → newline, `\t` → tab, `\r` → carriage return, and
/// `\0` → null. Any other `\X` sequence passes through `X` literally.
pub fn unescape_string(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        // The lexer ends no string content with a lone backslash, and the
        // parser admits only the escapes below (`\$` and `\#` are the
        // character itself).
        match chars.next().expect("a backslash escapes a character") {
            'n' => result.push('\n'),
            't' => result.push('\t'),
            'r' => result.push('\r'),
            '0' => result.push('\0'),
            // `\u{1F389}`.
            'u' => {
                let rest: String = chars.clone().collect();
                let (digits, _) = rest
                    .strip_prefix('{')
                    .and_then(|r| r.split_once('}'))
                    .expect("the parser admits only a well-formed unicode escape");
                result.extend(
                    u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32),
                );
                for _ in 0..digits.len() + 2 {
                    chars.next();
                }
            }
            other => result.push(other),
        }
    }
    result
}

/// Extract simple string content from a LITERAL or STRING_EXPR syntax node.
/// Walks children looking for STRING_CONTENT tokens and concatenates them;
/// a heredoc is dedented as in an expression, so a heredoc pattern matches
/// the same heredoc as a value.
pub fn extract_simple_string_content(node: &crate::SyntaxNode) -> String {
    let is_triple = node
        .children_with_tokens()
        .filter_map(|c| c.into_token())
        .find(|t| t.kind() == SyntaxKind::STRING_START)
        .is_some_and(|t| t.text().starts_with("\"\"\""));
    let contents: Vec<String> = node
        .children_with_tokens()
        .filter_map(|c| c.into_token())
        .filter(|t| t.kind() == SyntaxKind::STRING_CONTENT)
        .map(|t| {
            if is_triple {
                t.text().replace("\r\n", "\n")
            } else {
                t.text().to_string()
            }
        })
        .collect();
    if !is_triple {
        return contents.iter().map(|c| unescape_string(c)).collect();
    }
    let trim_level = contents
        .last()
        .and_then(|last| last.split('\n').next_back())
        .map(|line| line.chars().take_while(|c| *c == ' ' || *c == '\t').count())
        .unwrap_or(0);
    let count = contents.len();
    contents
        .iter()
        .enumerate()
        .map(|(i, text)| {
            apply_heredoc_content(unescape_string(text), i == 0, i + 1 == count, trim_level)
        })
        .collect()
}
