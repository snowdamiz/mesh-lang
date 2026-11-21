use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use mesh_common::span::{LineIndex, Span};

/// A file a diagnostic points into: its path and, when it was read, the exact
/// text its spans index. Cloning shares one copy. For a `.test.mpl` file the
/// text is the preprocessed program, which keeps every line and column.
#[derive(Clone)]
pub struct SourceFile(Arc<Inner>);

struct Inner {
    path: PathBuf,
    text: Option<Arc<str>>,
    lines: OnceLock<LineIndex>,
}

/// A 1-based line and a 1-based column counted in Unicode scalar values, as
/// rustc counts them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

impl SourceFile {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<Arc<str>>) -> Self {
        Self::with_text(path.into(), Some(text.into()))
    }

    /// A file known only by its path: the runtime's, or one that could not be read.
    pub fn unread(path: impl Into<PathBuf>) -> Self {
        Self::with_text(path.into(), None)
    }

    fn with_text(path: PathBuf, text: Option<Arc<str>>) -> Self {
        SourceFile(Arc::new(Inner {
            path,
            text,
            lines: OnceLock::new(),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn text(&self) -> Option<&str> {
        self.0.text.as_deref()
    }

    /// Where byte `offset` is. An offset inside a character counts as that
    /// character's start; `None` past the end or for an unread file.
    pub fn line_col(&self, offset: u32) -> Option<LineCol> {
        let (text, offset, line, start) = self.locate(offset)?;
        let col = text[start..offset].chars().count() as u32 + 1;
        Some(LineCol { line, col })
    }

    /// The LSP position of byte `offset`: a 0-based line and a column in
    /// UTF-16 code units.
    pub fn utf16_position(&self, offset: u32) -> Option<(u32, u32)> {
        let (text, offset, line, start) = self.locate(offset)?;
        let character = text[start..offset]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        Some((line - 1, character))
    }

    /// The byte offset of an LSP position. A `character` past the end of its
    /// line is the line's end, before any `\r\n` or `\n`; `None` past the last line.
    pub fn offset_of_utf16(&self, line: u32, character: u32) -> Option<u32> {
        let text = self.text()?;
        let start = self.lines()?.line_start(line.checked_add(1)?)? as usize;
        let rest = &text[start..];
        let content = rest.split('\n').next().unwrap_or(rest);
        let content = content.strip_suffix('\r').unwrap_or(content);
        let mut units = 0;
        for (index, c) in content.char_indices() {
            if units >= character {
                return Some((start + index) as u32);
            }
            units += c.len_utf16() as u32;
        }
        Some((start + content.len()) as u32)
    }

    /// `span` as a range a renderer can underline: inside the text, on
    /// character boundaries, and at least one character wide. A span at the
    /// end of the file (an unclosed string) covers the last character. An
    /// empty or unread file leaves an empty span empty.
    pub fn clamp(&self, span: Span) -> Span {
        let Some(text) = self.text() else {
            return span;
        };
        let len = text.len();
        let mut start = floor_char_boundary(text, span.start as usize);
        let mut end = floor_char_boundary(text, span.end as usize).max(start);
        if start == end {
            if start < len {
                end = (start + 1..=len)
                    .find(|&i| text.is_char_boundary(i))
                    .unwrap_or(len);
            } else if start > 0 {
                start = floor_char_boundary(text, start - 1);
            }
        }
        Span::new(start as u32, end as u32)
    }

    fn lines(&self) -> Option<&LineIndex> {
        let text = self.text()?;
        Some(self.0.lines.get_or_init(|| LineIndex::new(text)))
    }

    /// The text, `offset` floored to a character boundary, its 1-based line,
    /// and that line's start.
    fn locate(&self, offset: u32) -> Option<(&str, usize, u32, usize)> {
        let text = self.text()?;
        if offset as usize > text.len() {
            return None;
        }
        let offset = floor_char_boundary(text, offset as usize);
        let lines = self.lines()?;
        let (line, _) = lines.line_col(offset as u32);
        let start = lines.line_start(line)? as usize;
        Some((text, offset, line, start))
    }
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Two handles to one path are the same file.
impl PartialEq for SourceFile {
    fn eq(&self, other: &Self) -> bool {
        self.path() == other.path()
    }
}

impl Eq for SourceFile {}

impl fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SourceFile").field(&self.path()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SourceFile {
        SourceFile::new("main.mpl", text)
    }

    fn at(line: u32, col: u32) -> Option<LineCol> {
        Some(LineCol { line, col })
    }

    #[test]
    fn ascii_line_col() {
        let f = file("let x = 1\nlet y = 2\n");
        assert_eq!(f.line_col(0), at(1, 1));
        assert_eq!(f.line_col(4), at(1, 5));
        assert_eq!(f.line_col(9), at(1, 10), "the newline ends its own line");
        assert_eq!(f.line_col(10), at(2, 1));
        assert_eq!(
            f.line_col(20),
            at(3, 1),
            "end of file after a final newline"
        );
        assert_eq!(f.line_col(21), None);
    }

    #[test]
    fn columns_count_unicode_scalars() {
        // é is 2 bytes, → is 3, 🦀 is 4.
        let f = file("é→🦀x\n");
        assert_eq!(f.line_col(0), at(1, 1));
        assert_eq!(f.line_col(2), at(1, 2));
        assert_eq!(f.line_col(5), at(1, 3));
        assert_eq!(f.line_col(9), at(1, 4));
        assert_eq!(f.line_col(7), at(1, 3), "inside 🦀 counts as its start");
    }

    #[test]
    fn utf16_positions_count_code_units() {
        let f = file("é→🦀x\nb");
        assert_eq!(f.utf16_position(0), Some((0, 0)));
        assert_eq!(f.utf16_position(2), Some((0, 1)));
        assert_eq!(f.utf16_position(5), Some((0, 2)));
        assert_eq!(f.utf16_position(9), Some((0, 4)), "🦀 is a surrogate pair");
        assert_eq!(f.utf16_position(11), Some((1, 0)));
        assert_eq!(f.utf16_position(12), Some((1, 1)));
        assert_eq!(f.utf16_position(13), None);
    }

    #[test]
    fn utf16_positions_round_trip() {
        let f = file("é→🦀x\r\nb\n");
        for offset in [0, 2, 5, 9, 10, 12, 13, 14] {
            let (line, character) = f.utf16_position(offset).unwrap();
            assert_eq!(
                f.offset_of_utf16(line, character),
                Some(offset),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn utf16_offsets_clamp_to_the_line_content() {
        let f = file("ab\r\ncd\nef");
        assert_eq!(f.offset_of_utf16(0, 99), Some(2), "before \\r\\n");
        assert_eq!(f.offset_of_utf16(1, 99), Some(6), "before \\n");
        assert_eq!(f.offset_of_utf16(2, 99), Some(9), "end of file");
        assert_eq!(f.offset_of_utf16(3, 0), None);
    }

    #[test]
    fn crlf_lines() {
        let f = file("a\r\nb");
        assert_eq!(f.line_col(1), at(1, 2), "\\r belongs to the line it ends");
        assert_eq!(f.line_col(3), at(2, 1));
    }

    #[test]
    fn empty_and_unread_files() {
        let empty = file("");
        assert_eq!(empty.line_col(0), at(1, 1));
        assert_eq!(empty.utf16_position(0), Some((0, 0)));
        assert_eq!(empty.offset_of_utf16(0, 5), Some(0));
        assert_eq!(empty.clamp(Span::new(0, 0)), Span::new(0, 0));

        let unread = SourceFile::unread("src/main.mpl");
        assert_eq!(unread.text(), None);
        assert_eq!(unread.line_col(0), None);
        assert_eq!(unread.utf16_position(0), None);
        assert_eq!(unread.offset_of_utf16(0, 0), None);
        assert_eq!(unread.clamp(Span::new(3, 7)), Span::new(3, 7));
    }

    #[test]
    fn clamp_widens_and_aligns_spans() {
        let f = file("ab🦀");
        assert_eq!(f.clamp(Span::new(0, 2)), Span::new(0, 2));
        assert_eq!(
            f.clamp(Span::new(1, 1)),
            Span::new(1, 2),
            "an empty span covers one character"
        );
        assert_eq!(
            f.clamp(Span::new(2, 2)),
            Span::new(2, 6),
            "a whole multi-byte character"
        );
        assert_eq!(
            f.clamp(Span::new(3, 5)),
            Span::new(2, 6),
            "inside a character floors to its start"
        );
        assert_eq!(
            f.clamp(Span::new(6, 6)),
            Span::new(2, 6),
            "at the end, the last character"
        );
        assert_eq!(
            f.clamp(Span::new(4, 40)),
            Span::new(2, 6),
            "past the end stops at the end"
        );
    }

    #[test]
    fn equality_is_by_path() {
        assert_eq!(SourceFile::new("a.mpl", "x"), SourceFile::unread("a.mpl"));
        assert_ne!(SourceFile::new("a.mpl", "x"), SourceFile::new("b.mpl", "x"));
        assert_eq!(format!("{:?}", file("x")), "SourceFile(\"main.mpl\")");
    }
}
