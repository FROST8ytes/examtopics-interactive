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

fn page_lines(page: u32, raw: &RawPage) -> Vec<Line> {
    let mut items: Vec<&RawItem> = raw
        .items
        .iter()
        .filter(|it| !it.s.trim().is_empty() && it.h > 0.0)
        .collect();
    items.sort_by(|a, b| {
        (a.y + a.h / 2.0)
            .total_cmp(&(b.y + b.h / 2.0))
            .then(a.x.total_cmp(&b.x))
    });

    // Cluster items whose vertical centres are close to each other.
    let mut rows: Vec<Vec<&RawItem>> = Vec::new();
    for it in items {
        let centre = it.y + it.h / 2.0;
        let joined = rows.last_mut().and_then(|row| {
            let top = row.iter().map(|r| r.y).fold(f32::MAX, f32::min);
            let bottom = row.iter().map(|r| r.y + r.h).fold(f32::MIN, f32::max);
            let row_centre = (top + bottom) / 2.0;
            let tolerance = (bottom - top).min(it.h) * 0.5;
            ((centre - row_centre).abs() <= tolerance).then_some(row)
        });
        match joined {
            Some(row) => row.push(it),
            None => rows.push(vec![it]),
        }
    }

    rows.into_iter()
        .map(|mut row| {
            row.sort_by(|a, b| a.x.total_cmp(&b.x));
            let mut text = String::new();
            let mut prev_right: Option<f32> = None;
            for it in &row {
                if let Some(right) = prev_right {
                    let gap = it.x - right;
                    let needs_space = !text.ends_with(' ') && !it.s.starts_with(' ');
                    if needs_space && gap > it.h * 0.12 {
                        text.push(' ');
                    }
                }
                text.push_str(&it.s);
                prev_right = Some(it.x + it.w);
            }
            Line {
                page,
                top: row.iter().map(|r| r.y).fold(f32::MAX, f32::min),
                bottom: row.iter().map(|r| r.y + r.h).fold(f32::MIN, f32::max),
                left: row.iter().map(|r| r.x).fold(f32::MAX, f32::min),
                right: row.iter().map(|r| r.x + r.w).fold(f32::MIN, f32::max),
                text: normalize_spaces(&text),
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect()
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
                if !in_band(l, h) {
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
    fn page_number_detection() {
        assert!(is_page_number("12"));
        assert!(is_page_number("Page 3 of 40"));
        assert!(is_page_number("3 / 40"));
        assert!(!is_page_number("A. 12 hours"));
        assert!(!is_page_number("Topic 1"));
    }
}
