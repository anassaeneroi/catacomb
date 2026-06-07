//! Minimal WebVTT / SRT cue parser for the transcript viewer.
//!
//! The transcript viewer only needs each cue's **start time** and its
//! **text** — not end times, styling, regions, or positioning — so this is
//! deliberately small and tolerant of both WebVTT (`.vtt`) and SRT (`.srt`)
//! timestamps. The web UI parses the served `.vtt` in JavaScript; the desktop
//! reads the file off disk and uses this.

/// One subtitle cue: when it starts (seconds into the video) and its
/// tag-stripped text.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub text: String,
}

/// Parse VTT/SRT text into cues, in file order. Inline tags like `<c>` are
/// stripped and consecutive duplicate lines (common in auto-generated
/// captions, which re-emit the rolling line) are collapsed.
pub fn parse(text: &str) -> Vec<Cue> {
    let lines: Vec<&str> = text.lines().collect();
    let mut cues: Vec<Cue> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // A cue timing line is "<start> --> <end> [settings]".
        if let Some(arrow) = lines[i].find("-->") {
            if let Some(start) = parse_timestamp(&lines[i][..arrow]) {
                i += 1;
                let mut buf: Vec<String> = Vec::new();
                while i < lines.len() && !lines[i].trim().is_empty() {
                    buf.push(strip_tags(lines[i]));
                    i += 1;
                }
                let text: String = buf.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() && cues.last().map(|c| c.text.as_str()) != Some(text.as_str()) {
                    cues.push(Cue { start, text });
                }
                continue;
            }
        }
        i += 1;
    }
    cues
}

/// Strip simple `<...>` inline tags (e.g. `<c.colorE5E5E5>`, `<00:00:01.000>`)
/// from a caption line.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Parse a `HH:MM:SS.mmm` / `MM:SS.mmm` (WebVTT) or comma-decimal (SRT)
/// timestamp into seconds. Returns `None` for non-timestamp lines.
fn parse_timestamp(s: &str) -> Option<f64> {
    let token = s.trim().replace(',', ".");
    let token = token.split_whitespace().next()?; // drop trailing cue settings
    let mut secs = 0f64;
    let mut any = false;
    for part in token.split(':') {
        let v: f64 = part.parse().ok()?;
        secs = secs * 60.0 + v;
        any = true;
    }
    if any { Some(secs) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vtt_with_tags() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello <c>world</c>\n\n\
                   00:01:05.500 --> 00:01:07.000 align:start\nSecond line\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: 1.0, text: "Hello world".into() });
        assert_eq!(cues[1].start, 65.5);
        assert_eq!(cues[1].text, "Second line");
    }

    #[test]
    fn parses_srt_and_collapses_dupes() {
        let srt = "1\n00:00:02,000 --> 00:00:04,000\nsame\n\n\
                   2\n00:00:04,000 --> 00:00:06,000\nsame\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 1, "consecutive duplicate lines collapse");
        assert_eq!(cues[0].start, 2.0);
    }

    #[test]
    fn multi_line_cue_joins() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nline one\nline two\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "line one line two");
    }

    #[test]
    fn header_only_is_empty() {
        assert!(parse("WEBVTT\n\n").is_empty());
        assert!(parse("").is_empty());
    }
}
