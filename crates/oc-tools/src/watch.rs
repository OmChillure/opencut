//! What the director should look at before it cuts a range.

use crate::finish::is_filler;

/// Silence shorter than this is a breath, not a hole.
pub const SILENCE_GAP_SECS: f64 = 0.7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapKind {
    Filler,
    Silence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceGap {
    pub kind: GapKind,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Three looks across a range, or one look when the range is a single moment.
#[must_use]
pub fn watch_times(start: f64, end: f64) -> Vec<f64> {
    if !start.is_finite() || !end.is_finite() {
        return Vec::new();
    }
    let start = start.max(0.0);
    let end = end.max(start);
    let span = end - start;
    if span < 0.4 {
        return vec![start];
    }
    let mut times = vec![start + span * 0.15, start + span * 0.5, end - span * 0.15];
    times.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    times
}

/// Filler lines and silence holes in source order. `cues` are `(start, end, text)`.
#[must_use]
pub fn source_gaps(cues: &[(f64, f64, &str)], source_end: f64) -> Vec<SourceGap> {
    let mut cues: Vec<(f64, f64, &str)> = cues
        .iter()
        .copied()
        .filter(|(start, end, text)| end - start > 0.02 && !text.trim().is_empty() && *end > 0.0)
        .map(|(start, end, text)| (start.max(0.0), end.max(start), text))
        .collect();
    cues.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::new();
    let mut cursor = 0.0_f64;
    for (start, end, text) in cues {
        if start - cursor >= SILENCE_GAP_SECS {
            out.push(SourceGap {
                kind: GapKind::Silence,
                start: cursor,
                end: start,
                text: String::new(),
            });
        }
        if is_filler(text) {
            out.push(SourceGap {
                kind: GapKind::Filler,
                start,
                end,
                text: text.trim().to_string(),
            });
        }
        cursor = cursor.max(end);
    }
    let tail = source_end.max(cursor);
    if tail - cursor >= SILENCE_GAP_SECS {
        out.push(SourceGap {
            kind: GapKind::Silence,
            start: cursor,
            end: tail,
            text: String::new(),
        });
    }
    out
}

/// Gaps that overlap `[start, end]`.
#[must_use]
pub fn gaps_overlapping(gaps: &[SourceGap], start: f64, end: f64) -> Vec<SourceGap> {
    let start = start.max(0.0);
    let end = end.max(start);
    gaps.iter()
        .filter(|gap| gap.end > start + 0.05 && gap.start < end - 0.05)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_range_is_three_looks_and_a_moment_is_one() {
        let times = watch_times(10.0, 16.0);
        assert_eq!(times.len(), 3);
        assert!(times[0] > 10.0 && times[0] < times[1] && times[1] < times[2]);
        assert!(times[2] < 16.0);
        assert_eq!(watch_times(4.0, 4.2), vec![4.0]);
        assert_eq!(watch_times(8.0, 3.0), vec![8.0]);
        assert!(watch_times(f64::NAN, 4.0).is_empty());
        assert!(watch_times(1.0, f64::INFINITY).is_empty());
    }

    #[test]
    fn filler_and_a_hole_are_listed_in_order() {
        let cues = [
            (0.2, 1.0, "hello there"),
            (1.1, 1.6, "um uh"),
            (4.0, 5.0, "the point"),
        ];
        let gaps = source_gaps(&cues, 8.0);
        assert!(gaps.iter().any(|gap| {
            gap.kind == GapKind::Filler && gap.text == "um uh" && (gap.start - 1.1).abs() < 1e-6
        }));
        assert!(
            gaps.iter()
                .any(|gap| { gap.kind == GapKind::Silence && gap.start > 1.5 && gap.end > 3.5 })
        );
        assert!(
            gaps.iter()
                .any(|gap| { gap.kind == GapKind::Silence && (gap.end - 8.0).abs() < 1e-6 })
        );
        let window = gaps_overlapping(&gaps, 1.05, 1.55);
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].kind, GapKind::Filler);
        let hole = gaps_overlapping(&gaps, 2.0, 3.5);
        assert!(hole.iter().all(|gap| gap.kind == GapKind::Silence));
        assert!(source_gaps(&[(0.0, 1.0, "hi")], 1.2).is_empty());
        assert!(source_gaps(&[(0.0, 1.0, "um the point")], 1.2).is_empty());
    }
}
