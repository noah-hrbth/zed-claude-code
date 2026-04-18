use anyhow::{Context, Result};
use std::path::Path;

/// Derived 1-based, inclusive line range of a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

/// Given the file content, the selected text, and the current cursor row (1-based),
/// derive the best-guess line range of the selection.
///
/// Strategy: search for the exact selected text in the file. If it occurs in a single
/// location, use that. If it occurs multiple times, pick the occurrence whose endpoints
/// are closest to `cursor_row`. If the text is absent or empty, fall back to
/// "caret at tail of selection": `end = cursor_row`, `start = end - newline_count`.
pub fn derive(file_content: &str, selection: &str, cursor_row: u32) -> LineRange {
    let trimmed_sel = selection;
    if trimmed_sel.is_empty() {
        return LineRange {
            start: cursor_row,
            end: cursor_row,
        };
    }

    let newlines_in_sel = trimmed_sel.chars().filter(|c| *c == '\n').count() as u32;
    let span_lines = newlines_in_sel + 1;

    if let Some(range) = search_in_file(file_content, trimmed_sel, cursor_row, span_lines) {
        return range;
    }

    // Fallback: caret at tail of selection.
    let end = cursor_row.max(1);
    let start = end.saturating_sub(span_lines - 1).max(1);
    LineRange { start, end }
}

/// Convenience entry point that reads the file from disk.
pub fn derive_from_path(path: &Path, selection: &str, cursor_row: u32) -> Result<LineRange> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(derive(&content, selection, cursor_row))
}

fn search_in_file(
    content: &str,
    needle: &str,
    cursor_row: u32,
    span_lines: u32,
) -> Option<LineRange> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let mut matches: Vec<LineRange> = Vec::new();

    // Search for needle at every possible start-of-line position. This is O(N*M) in the
    // worst case but selections are small and files are typically <100k lines. Good enough.
    let mut byte_pos = 0usize;
    for (line_idx, line) in lines.iter().enumerate() {
        // Check all start offsets within this line.
        let line_start = byte_pos;
        // Try matching starting at line_start and at each char boundary inside the line
        // where a newline does not occur. We only emit matches aligned to line boundaries
        // on the start side (selections from editors are typically line-based, but a mid-line
        // start is still allowed and represented by the same start_line).
        for offset in 0..line.len() {
            let candidate_start = line_start + offset;
            if candidate_start + needle.len() > content.len() {
                break;
            }
            if !content.is_char_boundary(candidate_start) {
                continue;
            }
            if content[candidate_start..].starts_with(needle) {
                let start_line = (line_idx + 1) as u32;
                let end_line = start_line + span_lines - 1;
                matches.push(LineRange {
                    start: start_line,
                    end: end_line,
                });
            }
        }
        byte_pos += line.len();
    }

    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(matches[0]);
    }
    // Multiple matches: pick the one whose endpoints are closest to cursor_row.
    matches.into_iter().min_by_key(|r| {
        let d_start = (r.start as i64 - cursor_row as i64).abs();
        let d_end = (r.end as i64 - cursor_row as i64).abs();
        d_start.min(d_end)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "line 1\nline 2\nhello world\nline 4\nline 5\nhello world\nline 7\n";

    #[test]
    fn unique_match() {
        let r = derive(SAMPLE, "line 4", 4);
        assert_eq!(r, LineRange { start: 4, end: 4 });
    }

    #[test]
    fn multiline_unique() {
        let r = derive("a\nb\nc\nd\n", "b\nc", 2);
        assert_eq!(r, LineRange { start: 2, end: 3 });
    }

    #[test]
    fn duplicate_text_prefers_closest_to_cursor() {
        let r = derive(SAMPLE, "hello world", 6);
        assert_eq!(r, LineRange { start: 6, end: 6 });
        let r = derive(SAMPLE, "hello world", 3);
        assert_eq!(r, LineRange { start: 3, end: 3 });
    }

    #[test]
    fn fallback_when_text_missing() {
        let r = derive(SAMPLE, "not present\nin file", 10);
        // Caret-at-tail: end=10, start=10-1=9
        assert_eq!(r, LineRange { start: 9, end: 10 });
    }

    #[test]
    fn empty_selection_collapses_to_cursor() {
        let r = derive(SAMPLE, "", 3);
        assert_eq!(r, LineRange { start: 3, end: 3 });
    }
}
