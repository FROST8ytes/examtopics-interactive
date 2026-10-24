//! Turns positioned text items (as produced by pdf.js `getTextContent`) into
//! reading-order lines, and strips running headers / footers.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A page of text items. Coordinates are PDF points with the origin at the
/// top-left corner of the page (y grows downwards).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawPage {
    pub width: f32,
    pub height: f32,
    pub items: Vec<RawItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawItem {
    /// Text content.
    pub s: String,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Zero-based page index.
    pub page: u32,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    pub text: String,
    /// Several lines of the OCR layer were stacked on this one position, so the
    /// reading order (and even word order) cannot be trusted.
    pub stacked: bool,
}

impl Line {
    pub fn height(&self) -> f32 {
        (self.bottom - self.top).max(1.0)
    }
}

/// Page geometry needed to cut snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
}

/// Fraction of the page height treated as header / footer band.
const BAND: f32 = 0.07;

/// Groups the text items of every page into lines and removes running
/// headers, footers and page numbers.
pub fn build_lines(pages: &[RawPage]) -> Vec<Line> {
    let per_page: Vec<Vec<Line>> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| page_lines(i as u32, p))
        .collect();
    strip_running_text(pages, per_page)
}

/// Consecutive items (in content-stream order) that continue one line of text.
#[derive(Debug)]
struct Run {
    first: usize,
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
    text: String,
}

impl Run {
    fn centre(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }

    fn height(&self) -> f32 {
        (self.bottom - self.top).max(1.0)
    }

    fn overlaps(&self, other: &Run) -> bool {
        let shared = self.right.min(other.right) - self.left.max(other.left);
        shared
            > 0.3
                * (self.right - self.left)
                    .min(other.right - other.left)
                    .max(1.0)
    }

    /// Appends text, separated by a space unless the two pieces overlap
    /// (which means one word was split into several items).
    fn push(&mut self, text: &str, left: f32, right: f32, height: f32) {
        let gap = left - self.right;
        if !self.text.ends_with(' ') && !text.starts_with(' ') && gap > -0.6 * height {
            self.text.push(' ');
        }
        self.text.push_str(text);
        self.right = self.right.max(right);
    }
}

/// Builds lines from a page's text items.
///
/// Items are first chained into runs in content-stream order (OCR layers emit one item
/// per word), then runs are grouped into rows by vertical position. Runs on the same row
/// that overlap horizontally are separate lines: OCR software sometimes misplaces a whole
/// line on top of another, and sorting their words by x would interleave them.
fn page_lines(page: u32, raw: &RawPage) -> Vec<Line> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, it) in raw.items.iter().enumerate() {
        if it.s.trim().is_empty() || it.h <= 0.0 {
            continue;
        }
        let centre = it.y + it.h / 2.0;
        let continues = runs.last().is_some_and(|r| {
            (centre - r.centre()).abs() <= 0.5 * r.height().min(it.h)
                // OCR word boxes may overlap the previous word a little.
                && it.x >= r.right - it.h
                && it.x - r.right <= 3.0 * it.h
        });
        match runs.last_mut() {
            Some(r) if continues => {
                r.push(&it.s, it.x, it.x + it.w, it.h);
                r.top = r.top.min(it.y);
                r.bottom = r.bottom.max(it.y + it.h);
            }
            _ => runs.push(Run {
                first: i,
                top: it.y,
                bottom: it.y + it.h,
                left: it.x,
                right: it.x + it.w,
                text: it.s.clone(),
            }),
        }
    }
    runs.sort_by(|a, b| {
        a.centre()
            .total_cmp(&b.centre())
            .then(a.left.total_cmp(&b.left))
    });

    // Rows: runs whose vertical centres are close.
    let mut rows: Vec<Vec<Run>> = Vec::new();
    for run in runs {
        let joins = rows.last().is_some_and(|row| {
            let top = row.iter().map(|r| r.top).fold(f32::MAX, f32::min);
            let bottom = row.iter().map(|r| r.bottom).fold(f32::MIN, f32::max);
            let tolerance = (bottom - top).min(run.height()) * 0.5;
            (run.centre() - (top + bottom) / 2.0).abs() <= tolerance
        });
        if joins {
            rows.last_mut().unwrap().push(run);
        } else {
            rows.push(vec![run]);
        }
    }

    let mut lines = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.left.total_cmp(&b.left));
        // Split the row into lines whose runs don't overlap horizontally.
        let mut split: Vec<Vec<Run>> = Vec::new();
        for run in row {
            match split
                .iter_mut()
                .find(|l| l.iter().all(|r| !r.overlaps(&run)))
            {
                Some(line) => line.push(run),
                None => split.push(vec![run]),
            }
        }
        split.sort_by_key(|l| l.iter().map(|r| r.first).min().unwrap_or(0));
        // One misplaced line on top of another is recoverable; a whole paragraph
        // collapsed onto one position is not.
        let stacked = split.len() >= 3;
        for runs in split {
            let mut iter = runs.into_iter();
            let Some(mut acc) = iter.next() else { continue };
            for r in iter {
                acc.push(&r.text, r.left, r.right, r.height());
                acc.top = acc.top.min(r.top);
                acc.bottom = acc.bottom.max(r.bottom);
            }
            let text = normalize_spaces(&acc.text);
            if !text.is_empty() {
                lines.push(Line {
                    page,
                    top: acc.top,
                    bottom: acc.bottom,
                    left: acc.left,
                    right: acc.right,
                    text,
                    stacked,
                });
            }
        }
    }
    lines
}

fn normalize_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Key used to recognise the same running header on different pages.
fn running_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                '#'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn is_page_number(s: &str) -> bool {
    let k = running_key(s);
    let k = k.trim_start_matches("page");
    let parts: Vec<&str> = k.split(['/', '|']).flat_map(|p| p.split("of")).collect();
    !k.is_empty()
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c == '#' || c == '-' || c == '.'))
        && k.contains('#')
}

/// Question / topic headers and answer keys often sit at the very top of a page and,
/// with digits normalised, look like a repeated running header. Never strip them.
fn is_structural(text: &str) -> bool {
    let t = text.trim_start().to_ascii_lowercase();
    t.starts_with("question") || t.starts_with("topic") || t.starts_with("correct answer")
}

fn in_band(line: &Line, page_height: f32) -> bool {
    line.bottom <= page_height * BAND || line.top >= page_height * (1.0 - BAND)
}

fn strip_running_text(pages: &[RawPage], per_page: Vec<Vec<Line>>) -> Vec<Line> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (lines, raw) in per_page.iter().zip(pages) {
        let keys: HashSet<String> = lines
            .iter()
            .filter(|l| in_band(l, raw.height))
            .map(|l| running_key(&l.text))
            .collect();
        for k in keys {
            *counts.entry(k).or_default() += 1;
        }
    }
    let threshold = ((pages.len() as f32) * 0.4).ceil().max(3.0) as usize;

    per_page
        .into_iter()
        .zip(pages)
        .flat_map(|(lines, raw)| {
            let h = raw.height;
            let counts = &counts;
            lines.into_iter().filter(move |l| {
                if !in_band(l, h) || is_structural(&l.text) {
                    return true;
                }
                if is_page_number(&l.text) {
                    return false;
                }
                counts.get(&running_key(&l.text)).copied().unwrap_or(0) < threshold
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(s: &str, x: f32, y: f32) -> RawItem {
        RawItem {
            s: s.into(),
            x,
            y,
            w: s.len() as f32 * 5.0,
            h: 10.0,
        }
    }

    #[test]
    fn groups_words_into_lines() {
        let page = RawPage {
            width: 600.0,
            height: 800.0,
            items: vec![
                item("world", 40.0, 101.0),
                item("Hello", 10.0, 100.0),
                item("Next", 10.0, 120.0),
            ],
        };
        let lines = build_lines(&[page]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[1].text, "Next");
    }

    #[test]
    fn strips_headers_footers_and_page_numbers() {
        let pages: Vec<RawPage> = (0..5)
            .map(|i| RawPage {
                width: 600.0,
                height: 800.0,
                items: vec![
                    item("ExamTopics - Free Questions", 10.0, 10.0),
                    item(&format!("Body {i}"), 10.0, 300.0),
                    item(&format!("Page {} of 5", i + 1), 10.0, 780.0),
                ],
            })
            .collect();
        let lines = build_lines(&pages);
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|l| l.text.starts_with("Body")));
    }

    #[test]
    fn separates_touching_words_and_keeps_misplaced_lines_apart() {
        let w = |s: &str, x: f32, y: f32, width: f32| RawItem {
            s: s.into(),
            x,
            y,
            w: width,
            h: 8.0,
        };
        let page = RawPage {
            width: 600.0,
            height: 800.0,
            // Stream order as pdf.js reports it for an OCR layer: one item per word,
            // words touching, and a body line misplaced onto the header's row.
            items: vec![
                w("Question", 77.0, 26.0, 26.0),
                w("#50", 107.0, 26.0, 11.0),
                w("Topic", 515.0, 26.0, 19.0),
                w("1", 534.0, 26.0, 4.0),
                w("You", 77.0, 26.5, 14.0),
                w("have", 91.0, 26.5, 17.0),
                w("an", 108.0, 26.5, 9.0),
                w("Azure", 117.0, 26.5, 20.0),
                w("subscription.", 137.0, 26.5, 400.0),
            ],
        };
        let lines = build_lines(&[page]);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Question #50 Topic 1", "You have an Azure subscription."]
        );
    }

    #[test]
    fn joins_slightly_overlapping_ocr_word_boxes() {
        let w = |s: &str, x: f32, width: f32| RawItem {
            s: s.into(),
            x,
            y: 25.7,
            w: width,
            h: 8.8,
        };
        let page = RawPage {
            width: 612.0,
            height: 792.0,
            items: vec![
                w("Question", 77.4, 35.3),
                w("#2", 107.5, 9.8),
                w("Topic", 515.2, 21.6),
                w("2", 532.0, 6.3),
            ],
        };
        let lines = build_lines(&[page]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Question #2 Topic 2");
        assert!(!lines[0].stacked);
    }

    #[test]
    fn keeps_question_headers_at_the_top_of_many_pages() {
        let pages: Vec<RawPage> = (0..6)
            .map(|i| RawPage {
                width: 600.0,
                height: 800.0,
                items: vec![
                    item(&format!("Question #{} Topic 1", i + 1), 10.0, 20.0),
                    item(&format!("Body {i}"), 10.0, 300.0),
                ],
            })
            .collect();
        let lines = build_lines(&pages);
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.text.starts_with("Question #"))
                .count(),
            6
        );
    }

    #[test]
    fn page_number_detection() {
        assert!(is_page_number("12"));
        assert!(is_page_number("Page 3 of 40"));
        assert!(is_page_number("3 / 40"));
        assert!(!is_page_number("A. 12 hours"));
        assert!(!is_page_number("Topic 1"));
    }
}
