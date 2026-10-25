//! Rebuilds the "Answer Area" of hotspot and drag & drop questions from the OCR text
//! of the picture, so they can be answered interactively.
//!
//! A hotspot answer area is a column of labels ("To triage security alerts:"), each
//! followed by the options of its dropdown, which are left-aligned to the right of it.
//! A drag & drop answer area has the items in a left pane ("Methodology", "Actions")
//! and the targets in the right pane ("Answer Area"). OCR word positions are noisy,
//! but the left edges of runs of words and the reading order are reliable enough.

use crate::layout::Line;
use crate::model::{DragSlot, DropdownGroup};
use regex::Regex;
use std::sync::LazyLock;

/// A run of words from the answer area picture.
#[derive(Debug, Clone, PartialEq)]
struct Piece {
    text: String,
    left: f32,
    right: f32,
    top: f32,
    /// The left edge is a guess (split off a label on the same run).
    guessed_left: bool,
}

static ANSWER_AREA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(hot\s*area|answer\s*area|select\s*and\s*place)\s*:?\s*$").unwrap()
});
static AREA_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(.{0,40}?)\s*answer\s*area\s*$").unwrap());
static CORRECT_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(correct|suggested)\s*answer\s*[:;.]?\s*").unwrap());
static BOX_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*box\s*(\d{1,2})\s*[:;\-.]\s*(.*)$").unwrap());
static ORDERED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bin\s+sequence\b|\bcorrect\s+order\b|\barrange\s+them\b|\bin\s+order\b")
        .unwrap()
});
static COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(two|three|four|five|six|seven|eight|[2-8])\s+(actions|steps|objectives|tasks|cmdlets|commands|activities|phases|items)\b|\bwhich\s+(two|three|four|five|six|seven|eight|[2-8])\b").unwrap()
});

/// "Methodology  Answer Area": the header row of a drag & drop picture.
pub fn is_header(text: &str) -> bool {
    AREA_HEADER.is_match(text) && !ANSWER_AREA.is_match(text)
}

/// Two-letter words that are real text, not the remains of a dropdown arrow.
const SHORT_WORDS: &[&str] = &["No", "no", "or", "to", "of", "in", "on", "by"];

fn is_junk_token(t: &str) -> bool {
    let alnum = t.chars().filter(|c| c.is_alphanumeric()).count();
    if alnum == 0 {
        return true;
    }
    let word: String = t.chars().filter(|c| c.is_alphanumeric()).collect();
    match word.chars().count() {
        // A lone letter is the dropdown arrow ("v", "V", "W"); a lone digit is a value.
        1 => !word.chars().all(|c| c.is_ascii_digit()),
        // "AD", "ID", "IT" are real words; "Vv", "iy", "ba" are arrows.
        2 => {
            !(word.chars().all(|c| c.is_ascii_uppercase())
                || word.chars().all(|c| c.is_ascii_digit())
                || SHORT_WORDS.contains(&word.as_str()))
        }
        _ => false,
    }
}

/// Removes the debris that OCR finds around dropdowns and boxes: arrows ("v", "¥"),
/// borders ("|", "[") and stray quotes. Returns `None` when nothing real is left.
fn clean(text: &str) -> Option<String> {
    let mut tokens: Vec<&str> = text.split_whitespace().collect();
    while tokens.last().is_some_and(|t| is_junk_token(t)) {
        tokens.pop();
    }
    // Leading words are kept ("A managed identity", "To connect"); only symbols go.
    while tokens
        .first()
        .is_some_and(|t| !t.chars().any(char::is_alphanumeric))
    {
        tokens.remove(0);
    }
    let joined = tokens.join(" ");
    let trimmed = joined
        .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '(' && c != '.')
        .trim_end_matches(|c: char| {
            matches!(
                c,
                '|' | ']' | '[' | '!' | '_' | '~' | ',' | '"' | '\'' | '‘' | '’'
            )
        })
        .trim();
    let letters = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
    (letters >= 2 || trimmed.chars().any(|c| c.is_ascii_digit())).then(|| trimmed.to_string())
}

fn alnum(s: &str) -> usize {
    s.chars().filter(|c| c.is_alphanumeric()).count()
}

fn pieces(lines: &[&Line]) -> Vec<Piece> {
    let mut out = Vec::new();
    for line in lines {
        if ANSWER_AREA.is_match(&line.text) {
            continue;
        }
        // OCR sometimes breaks one entry into runs that nearly touch.
        let mut spans: Vec<(f32, f32, String)> = Vec::new();
        for span in &line.spans {
            match spans.last_mut() {
                Some(last) if span.left - last.1 < line.height() * 0.4 => {
                    last.1 = last.1.max(span.right);
                    last.2.push(' ');
                    last.2.push_str(&span.text);
                }
                _ => spans.push((span.left, span.right, span.text.clone())),
            }
        }
        let mut after_label = false;
        for (left, right, raw) in spans {
            // The answer picture's first row can share a line with "Correct Answer:".
            let raw = CORRECT_PREFIX.replace(&raw, "").into_owned();
            let Some(text) = clean(&raw) else {
                continue;
            };
            // Short leftovers are the dropdown arrow ("VF", "7"), which sits on the
            // label's row, or other debris; "No" is a real option.
            if alnum(&text) <= 2
                && text != "No"
                && (after_label || !text.chars().all(|c| c.is_ascii_digit()))
            {
                continue;
            }
            // Text removed by `clean` no longer counts towards the width.
            let kept = alnum(&text) as f32 / alnum(&raw).max(1) as f32;
            let right = left + (right - left) * kept.min(1.0);
            after_label |= text.ends_with(':');
            out.push(Piece {
                text,
                left,
                right,
                top: line.top,
                guessed_left: false,
            });
        }
    }
    out
}

/// The left edge shared by most entries: where the dropdown lists start.
fn option_column<'a>(pieces: impl Iterator<Item = &'a Piece>) -> Option<f32> {
    let lefts: Vec<f32> = pieces.map(|p| p.left).collect();
    lefts
        .iter()
        .map(|&x| (x, lefts.iter().filter(|&&o| (o - x).abs() <= 6.0).count()))
        .filter(|&(_, n)| n >= 2)
        // Most entries; on a tie, the leftmost column.
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.total_cmp(&a.0)))
        .map(|(x, _)| x)
}

/// Splits "To implement access requests, use: An access package" (OCR ran the label
/// into the first option) into the label and the option.
fn split_labels(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out = Vec::with_capacity(pieces.len());
    for p in pieces {
        match p.text.split_once(": ") {
            Some((label, rest)) if !label.is_empty() => match clean(rest) {
                Some(rest) if alnum(&rest) > 2 => {
                    out.push(Piece {
                        text: format!("{label}:"),
                        ..p.clone()
                    });
                    out.push(Piece {
                        text: rest,
                        left: p.right - 1.0,
                        guessed_left: true,
                        ..p
                    });
                }
                _ => out.push(Piece {
                    text: format!("{label}:"),
                    ..p
                }),
            },
            _ => out.push(p),
        }
    }
    out
}

/// Dropdown groups (label + options) of a hotspot answer area. Keys are left unset.
pub fn dropdown_groups(area: &[&Line]) -> Option<Vec<DropdownGroup>> {
    let pieces = split_labels(pieces(area));
    // OCR often reads the label's colon as a semicolon.
    let is_label_end = |p: &Piece| p.text.ends_with([':', ';']);
    // The dropdown lists are left-aligned on a column; labels sit to its left.
    let column = option_column(
        pieces
            .iter()
            .filter(|p| !is_label_end(p) && !p.guessed_left),
    )?;
    let is_label: Vec<bool> = pieces
        .iter()
        .map(|p| is_label_end(p) || (!p.guessed_left && p.left < column - 15.0))
        .collect();

    let mut groups: Vec<(Vec<String>, Vec<&Piece>)> = Vec::new();
    for (p, &label) in pieces.iter().zip(&is_label) {
        if label {
            match groups.last_mut() {
                Some((l, opts)) if opts.is_empty() => l.push(p.text.clone()),
                _ => groups.push((vec![p.text.clone()], Vec::new())),
            }
        } else {
            match groups.last_mut() {
                Some((_, opts)) => opts.push(p),
                // Options before any label: OCR misplaced the first label.
                None => groups.push((Vec::new(), vec![p])),
            }
        }
    }

    let groups = split_merged_groups(groups);
    let mut out = Vec::with_capacity(groups.len());
    for (i, (label, options)) in groups.into_iter().enumerate() {
        if !(2..=12).contains(&options.len()) {
            return None;
        }
        // The options of a dropdown list are left-aligned.
        let lefts: Vec<f32> = options
            .iter()
            .filter(|o| !o.guessed_left)
            .map(|o| o.left)
            .collect();
        let min = lefts.iter().copied().fold(f32::MAX, f32::min);
        let max = lefts.iter().copied().fold(f32::MIN, f32::max);
        if lefts.len() >= 2 && max - min > 12.0 {
            return None;
        }
        let mut label = label.join(" ");
        if let Some(l) = label.strip_suffix(';') {
            label = format!("{l}:");
        }
        out.push(DropdownGroup {
            label: if label.is_empty() {
                format!("Box {}", i + 1)
            } else {
                label
            },
            options: options.into_iter().map(|o| o.text.clone()).collect(),
            correct: None,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// OCR sometimes stacks two labels and then both option lists under them. When a
/// group's options start over ("DAST, SAST, DAST, SAST"), split it in two.
fn split_merged_groups(groups: Vec<(Vec<String>, Vec<&Piece>)>) -> Vec<(Vec<String>, Vec<&Piece>)> {
    let mut out = Vec::with_capacity(groups.len());
    for (mut labels, options) in groups {
        let restart = options
            .iter()
            .skip(2)
            .position(|o| similarity(&o.text, &options[0].text) > 0.9);
        match restart {
            Some(k) => {
                let (a, b) = options.split_at(k + 2);
                let second = if labels.len() >= 2 {
                    labels.pop().into_iter().collect()
                } else {
                    Vec::new()
                };
                out.push((labels, a.to_vec()));
                out.push((second, b.to_vec()));
            }
            None => out.push((labels, options)),
        }
    }
    out
}

/// A drag & drop answer area: items, target labels and whether it is an ordering task.
#[derive(Debug, Clone, PartialEq)]
pub struct DragArea {
    pub items: Vec<String>,
    pub slots: Vec<DragSlot>,
    pub ordered: bool,
}

/// Reads the items pane and the answer area pane of a drag & drop picture.
/// `area` starts at the header row ("Methodology  Answer Area").
pub fn drag_drop(area: &[&Line], body: &str) -> Option<DragArea> {
    let (h, header) = area
        .iter()
        .enumerate()
        .find(|(_, l)| AREA_HEADER.is_match(&l.text))?;
    // The answer pane starts where its heading does.
    let heading = header.spans.iter().find(|s| {
        s.text
            .to_lowercase()
            .replace(' ', "")
            .contains("answerarea")
    })?;
    let boundary = if heading.left - header.left > 40.0 {
        heading.left - 8.0
    } else {
        // "Actions AnswerArea" read as one run: estimate from the text length.
        let lower = header.text.to_lowercase();
        let at = lower.find("answer")? as f32 / lower.len().max(1) as f32;
        header.left + (header.right - header.left) * at - 8.0
    };
    if boundary <= header.left + 20.0 {
        return None;
    }
    let mut items = Vec::new();
    let mut targets: Vec<Piece> = Vec::new();
    for mut p in pieces(&area[h + 1..]) {
        // Pane headings and stray words are short; real entries have a word of 2+ letters.
        if !p
            .text
            .split_whitespace()
            .any(|w| w.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        {
            continue;
        }
        if p.left < boundary {
            // Icons in front of an item come out as "3}" or "i".
            let words: Vec<&str> = p.text.split_whitespace().collect();
            let skip = words
                .iter()
                .take_while(|w| alnum(w) <= 1 && **w != "A" && **w != "a")
                .count();
            if skip > 0 && skip < words.len() {
                p.text = words[skip..].join(" ");
            }
            items.push(p.text);
        } else {
            targets.push(p);
        }
    }
    let targets = target_labels(targets);
    if items.len() < 2 {
        return None;
    }
    // Ordering tasks have no target labels, just an empty answer area.
    let ordered = ORDERED.is_match(body) || targets.is_empty();
    let slots: Vec<DragSlot> = if ordered {
        let n = expected_steps(body).filter(|&n| n <= items.len())?;
        (1..=n)
            .map(|i| DragSlot {
                label: format!("Step {i}"),
                correct: None,
            })
            .collect()
    } else {
        targets
            .into_iter()
            .map(|label| DragSlot {
                label,
                correct: None,
            })
            .collect()
    };
    Some(DragArea {
        items,
        slots,
        ordered,
    })
}

/// Target labels of the answer pane. Labels that end with a colon may wrap onto
/// several lines; text to the right of them is the drop boxes' column.
fn target_labels(targets: Vec<Piece>) -> Vec<String> {
    let ends = |p: &Piece| p.text.ends_with([':', ';']);
    let Some(boxes) = targets
        .iter()
        .filter(|p| ends(p))
        .map(|p| p.right)
        .min_by(f32::total_cmp)
    else {
        return targets.into_iter().map(|p| p.text).collect();
    };
    let mut out: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for p in targets.into_iter().filter(|p| p.left < boxes) {
        let done = ends(&p);
        pending.push(p.text);
        if done {
            let mut label = pending.join(" ");
            if let Some(l) = label.strip_suffix(';') {
                label = format!("{l}:");
            }
            out.push(label);
            pending.clear();
        }
    }
    if !pending.is_empty() {
        out.push(pending.join(" "));
    }
    out
}

/// "Which three actions should you perform in sequence?" → 3.
fn expected_steps(body: &str) -> Option<usize> {
    let c = COUNT.captures(body)?;
    let word = c.get(1).or_else(|| c.get(3))?.as_str().to_lowercase();
    Some(match word.as_str() {
        "two" | "2" => 2,
        "three" | "3" => 3,
        "four" | "4" => 4,
        "five" | "5" => 5,
        "six" | "6" => 6,
        "seven" | "7" => 7,
        _ => 8,
    })
}

/// "Box 1: Azure Purview -" lines of the answer section, as (box number, answer).
pub fn box_answers(lines: &[&Line]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(c) = BOX_LINE.captures(&line.text) else {
            continue;
        };
        let Ok(n) = c[1].parse::<usize>() else {
            continue;
        };
        let mut text = c[2].trim().trim_end_matches(['-', '.', ' ']).to_string();
        if text.is_empty()
            && let Some(next) = lines.get(i + 1)
        {
            text = next
                .text
                .trim()
                .trim_end_matches(['-', '.', ' '])
                .to_string();
        }
        if !text.is_empty() && !out.iter().any(|(m, _)| *m == n) {
            out.push((n, text));
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Dice coefficient of character bigrams, ignoring case, spaces and punctuation.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    if a == b {
        return 1.0;
    }
    let grams = |s: &str| -> Vec<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (ga, mut gb) = (grams(&a), grams(&b));
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let total = ga.len() + gb.len();
    let mut shared = 0;
    for g in ga {
        if let Some(i) = gb.iter().position(|x| *x == g) {
            gb.swap_remove(i);
            shared += 1;
        }
    }
    2.0 * shared as f32 / total as f32
}

/// Index of the entry that best matches `text`, if it is a convincing match.
pub fn best_match(text: &str, entries: &[String]) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| (i, similarity(text, e)))
        .filter(|(_, s)| *s >= 0.8)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Fills dropdown keys from "Box N: answer" lines. Returns whether every group got one.
pub fn key_dropdowns(groups: &mut [DropdownGroup], boxes: &[(usize, String)]) -> bool {
    if boxes.len() != groups.len() {
        return false;
    }
    let keys: Option<Vec<usize>> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let (_, text) = boxes.iter().find(|(n, _)| *n == i + 1)?;
            best_match(text, &g.options)
        })
        .collect();
    match keys {
        Some(keys) => {
            for (g, k) in groups.iter_mut().zip(keys) {
                g.correct = Some(k);
            }
            true
        }
        None => false,
    }
}

/// Fills drag & drop keys from "Box N: answer" lines, or from the OCR text of the
/// answer picture (target label followed by the item placed on it, or the items in order).
pub fn key_drag_drop(area: &mut DragArea, boxes: &[(usize, String)], answer: &[&Line]) -> bool {
    let n = area.slots.len();
    if boxes.len() == n && (1..=n).all(|i| boxes.iter().any(|(m, _)| *m == i)) {
        for (i, slot) in area.slots.iter_mut().enumerate() {
            let text = &boxes.iter().find(|(m, _)| *m == i + 1).unwrap().1;
            // The explanation names the item; OCR may have missed it in the list.
            let k = best_match(text, &area.items).unwrap_or_else(|| {
                area.items.push(text.clone());
                area.items.len() - 1
            });
            slot.correct = Some(k);
        }
        return true;
    }

    let found = pieces(answer);
    let keys: Option<Vec<usize>> = if area.ordered {
        let placed: Vec<usize> = found
            .iter()
            .filter_map(|p| best_match(&p.text, &area.items))
            .collect();
        (placed.len() == n).then_some(placed)
    } else {
        area.slots
            .iter()
            .map(|slot| {
                let label = normalize(&slot.label);
                // The label and its item may have been read as one run.
                if let Some(p) = found.iter().find(|p| {
                    let t = normalize(&p.text);
                    t.len() > label.len() + 3 && t.starts_with(&label)
                }) {
                    let words = slot.label.split_whitespace().count();
                    let rest: Vec<&str> = p.text.split_whitespace().skip(words).collect();
                    return best_match(&rest.join(" "), &area.items);
                }
                let at = found
                    .iter()
                    .position(|p| similarity(&p.text, &slot.label) >= 0.75)?;
                let next = found.get(at + 1).filter(|p| p.top - found[at].top < 4.0)?;
                best_match(&next.text, &area.items)
            })
            .collect()
    };
    match keys {
        Some(keys) => {
            for (s, k) in area.slots.iter_mut().zip(keys) {
                s.correct = Some(k);
            }
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Span;

    /// A line made of spans given as (left, text).
    fn line(top: f32, spans: &[(f32, &str)]) -> Line {
        let spans: Vec<Span> = spans
            .iter()
            .map(|(x, t)| Span {
                left: *x,
                right: x + t.len() as f32 * 4.5,
                text: t.to_string(),
            })
            .collect();
        Line {
            page: 0,
            top,
            bottom: top + 9.0,
            left: spans[0].left,
            right: spans.last().unwrap().right,
            text: spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            stacked: false,
            spans,
        }
    }

    #[test]
    fn cleans_ocr_debris() {
        assert_eq!(clean("Standard user: W").as_deref(), Some("Standard user:"));
        assert_eq!(
            clean("[Specialized security").as_deref(),
            Some("Specialized security")
        );
        assert_eq!(clean("Azure AD").as_deref(), Some("Azure AD"));
        assert_eq!(clean("v"), None);
        assert_eq!(clean("¥."), None);
        assert_eq!(clean("I I I I"), None);
        assert_eq!(clean("2").as_deref(), Some("2"));
    }

    #[test]
    fn builds_dropdowns_from_the_answer_area() {
        // Question 4 of SC-100: OCR put the first two options on the label's row.
        let lines = [
            line(150.0, &[(85.0, "Answer Area")]),
            line(183.0, &[(106.0, "To connect the Azure data sources to")]),
            line(
                196.0,
                &[
                    (127.0, "Microsoft Information Protection:"),
                    (270.0, "Azure Purview"),
                    (413.0, "v"),
                ],
            ),
            line(196.0, &[(271.0, "Endpoint data loss prevention")]),
            line(238.0, &[(271.0, "Microsoft Defender for Cloud Apps")]),
            line(251.0, &[(271.0, "Microsoft Information Protection")]),
            line(270.0, &[(124.0, "To triage security alerts related to")]),
            line(284.0, &[(133.0, "resources that contain PII data:")]),
            line(299.0, &[(271.0, "Azure Monitor")]),
            line(312.0, &[(271.0, "Endpoint data loss prevention")]),
            line(326.0, &[(271.0, "Microsoft Defender for Cloud")]),
            line(339.0, &[(271.0, "Microsoft Defender for Cloud Apps")]),
        ];
        let refs: Vec<&Line> = lines.iter().collect();
        let groups = dropdown_groups(&refs).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].label,
            "To connect the Azure data sources to Microsoft Information Protection:"
        );
        assert_eq!(groups[0].options.len(), 4);
        assert_eq!(groups[0].options[0], "Azure Purview");
        assert_eq!(
            groups[1].label,
            "To triage security alerts related to resources that contain PII data:"
        );
        assert_eq!(groups[1].options[2], "Microsoft Defender for Cloud");

        let mut groups = groups;
        let boxes = vec![
            (1, "Azure Purview".to_string()),
            (2, "Microsoft Defender for Cloud".to_string()),
        ];
        assert!(key_dropdowns(&mut groups, &boxes));
        assert_eq!(groups[0].correct, Some(0));
        assert_eq!(groups[1].correct, Some(2));
    }

    #[test]
    fn reads_drag_and_drop_panes() {
        let lines = [
            line(201.0, &[(91.0, "Methodology"), (260.0, "Answer Area")]),
            line(
                231.0,
                &[(91.0, "Business continuity"), (280.0, "Assume breach ! :")],
            ),
            line(253.0, &[(385.0, "I I I I I I I I")]),
            line(
                258.0,
                &[
                    (91.0, "Data classification"),
                    (290.0, "Verify explicitly |"),
                ],
            ),
            line(286.0, &[(90.0, "Just-in-time (JIT) access .")]),
            line(292.0, &[(263.0, "Use least privilege access |")]),
            line(315.0, &[(90.0, "Segmenting access")]),
        ];
        let refs: Vec<&Line> = lines.iter().collect();
        let mut area = drag_drop(
            &refs,
            "Drag the appropriate methodologies to the correct principles.",
        )
        .unwrap();
        assert_eq!(
            area.items,
            [
                "Business continuity",
                "Data classification",
                "Just-in-time (JIT) access",
                "Segmenting access"
            ]
        );
        let labels: Vec<&str> = area.slots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Assume breach",
                "Verify explicitly",
                "Use least privilege access"
            ]
        );
        assert!(!area.ordered);

        let answer = [
            line(
                574.0,
                &[(256.0, "Assume breach"), (338.0, "Segmenting access")],
            ),
            line(
                604.0,
                &[(256.0, "Verify explicitly"), (338.0, "Data classification")],
            ),
            line(
                637.0,
                &[
                    (212.0, "Use least privilege access"),
                    (336.0, "Just-in-time (JIT) access"),
                ],
            ),
        ];
        let refs: Vec<&Line> = answer.iter().collect();
        assert!(key_drag_drop(&mut area, &[], &refs));
        let keys: Vec<Option<usize>> = area.slots.iter().map(|s| s.correct).collect();
        assert_eq!(keys, [Some(3), Some(1), Some(2)]);
    }

    #[test]
    fn ordering_tasks_get_numbered_steps() {
        let lines = [
            line(444.0, &[(79.0, "Actions AnswerArea")]),
            line(
                453.0,
                &[(80.0, "| Establish ransomware recovery readiness.")],
            ),
            line(
                463.0,
                &[(82.0, "Enable additional protection and detection controls.")],
            ),
            line(482.0, &[(80.0, "| implement disaster recovery.")]),
            line(492.0, &[(80.0, "| Enable automation.")]),
        ];
        let refs: Vec<&Line> = lines.iter().collect();
        // The header was read as one run, so the pane boundary is estimated.
        let mut header = lines[0].clone();
        header.right = 340.0;
        header.spans[0].right = 340.0;
        let refs: Vec<&Line> = std::iter::once(&header)
            .chain(refs[1..].iter().copied())
            .collect();
        let area = drag_drop(
            &refs,
            "Which three deployment objectives should you prioritize in sequence?",
        )
        .unwrap();
        assert!(area.ordered);
        assert_eq!(area.items.len(), 4);
        assert_eq!(area.slots.len(), 3);
        assert_eq!(area.slots[0].label, "Step 1");
    }

    #[test]
    fn matches_box_answers_loosely() {
        assert!(similarity("The Log Analytics agent", "TheLog Analyticsagent") > 0.95);
        let opts: Vec<String> = [
            "Microsoft Defender for Cloud",
            "Microsoft Defender for Cloud Apps",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(best_match("Microsoft Defender for Cloud", &opts), Some(0));
        assert_eq!(
            best_match("Microsoft Defender for Cloud Apps", &opts),
            Some(1)
        );
        assert_eq!(best_match("Azure Monitor", &opts), None);
    }
}
