//! Parser for ExamTopics PDFs (after OCR).
//!
//! The PDF layout is roughly:
//!
//! ```text
//! Topic 1 - Question Set 1
//! Question #12                                  Topic 1
//! HOTSPOT -                                     (optional type marker)
//! Introductory Info / Case study ... Question   (optional, case studies)
//! <question text>
//! A. <option>                                   (choice questions)
//! B. <option>
//! Hot Area: / Select and Place: / Answer Area   (image questions)
//! Correct Answer: B
//! Explanation: ... Reference: https://...
//! Community vote distribution
//! B (78%) C (22%)
//! ```
//!
//! Text is kept exactly as recognised; only line breaks are re-flowed into
//! paragraphs. Everything that cannot be represented as text (exhibits, hot
//! areas) is captured through page regions so the UI can show snapshots.

use crate::layout::{Line, PageInfo, RawPage, build_lines};
use crate::model::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// A vertical slice of a page, in PDF points from the top.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub page: u32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedQuestion {
    pub question: Question,
    pub question_regions: Vec<Region>,
    pub answer_regions: Vec<Region>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicInfo {
    pub number: u32,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParseOutput {
    /// Text found before the first question (usually the exam name).
    pub title: Option<String>,
    /// Exam code such as `AZ-104` guessed from the title.
    pub exam_code: Option<String>,
    pub topics: Vec<TopicInfo>,
    pub questions: Vec<ParsedQuestion>,
    pub pages: Vec<PageInfo>,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(
    QUESTION_HEADER,
    r"(?i)^\s*question\s*(#|no\.?|:)?\s*:?\s*(\d{1,4})\b\s*(.*)$"
);
re!(TOPIC_IN_HEADER, r"(?i)^[\s\-–—|,]*topic\s*(\d{1,3})\s*$");
re!(
    TOPIC_LINE,
    r"(?i)^\s*topic\s*(\d{1,3})\b\s*[-–—:,.]?\s*(.{0,100})$"
);
re!(
    CORRECT_ANSWER,
    r"(?i)^\s*(correct|suggested)\s*answer\s*[:;.]?\s*(.*)$"
);
re!(
    ANSWER_LETTERS,
    r"^\s*([A-H](?:[\s,&]*[A-H])*)(?:[^A-Za-z]|$)"
);
re!(COMMUNITY, r"(?i)community\s+vote");
re!(VOTE, r"\b([A-H]{1,8})\s*\(\s*(\d{1,3})\s*%\s*\)");
re!(EXPLANATION, r"(?i)^\s*explanation\s*[:/]?\s*(.*)$");
re!(REFERENCE, r"(?i)^\s*references?\s*:");
re!(URL, r"https?://\S+");
re!(
    TYPE_MARKER,
    r"(?i)^\s*(hot\s*spot|drag\s*(?:and\s*|&\s*|-\s*)?drop|simulation|lab)\s*[-–—:]?\s*$"
);
re!(
    AREA_MARKER,
    r"(?i)^\s*(hot\s*area|answer\s*area|select\s*and\s*place)\s*[:\-]?\s*(.*)$"
);
re!(INTRO_INFO, r"(?i)^\s*introductory\s+info");
re!(BARE_QUESTION, r"(?i)^\s*question\s*$");
re!(OPTION, r"^\s*([A-H])\s*[.)\]:,]\s*(.*)$");
re!(
    CHOOSE_N,
    r"(?i)\b(choose|select|which)\s+(two|three|four|five|2|3|4|5)\b"
);
re!(
    EXHIBIT,
    r"(?i)\bexhibit\b|\(click the .{1,30} tab\)|shown in the following (table|image|graphic)"
);
re!(SELECT_YES, r"(?i)select\s+yes\s+if");
re!(BOX_ANSWER, r"(?i)\bbox\s*(\d{1,2})\s*[:\-.]\s*(yes|no)\b");
re!(EXAM_CODE, r"\b([A-Z]{2,6}-?[0-9]{2,4}[A-Z]?)\b");
re!(
    YES_NO_ONLY,
    r"(?i)^\s*(statements?|yes|no|yes\s+no|answer\s*area)\s*$"
);
re!(TRAILING_YES_NO, r"(?i)\s+yes\s+no\s*$");

/// Parses the text layer of an ExamTopics PDF.
pub fn parse_pages(pages: &[RawPage]) -> ParseOutput {
    let infos: Vec<PageInfo> = pages
        .iter()
        .map(|p| PageInfo {
            width: p.width,
            height: p.height,
        })
        .collect();
    let lines = build_lines(pages);
    let mut out = parse_lines(&lines, &infos);
    out.pages = infos;
    out
}

/// Parses already-built lines. `pages` is used for region bounds.
pub fn parse_lines(lines: &[Line], pages: &[PageInfo]) -> ParseOutput {
    let mut out = ParseOutput::default();

    // Pull out topic headings ("Topic 2 - Case Study 1") so they don't end up in bodies.
    let mut kept: Vec<&Line> = Vec::with_capacity(lines.len());
    for line in lines {
        if let Some(c) = TOPIC_LINE.captures(&line.text)
            && line.text.len() <= 110
            && !QUESTION_HEADER.is_match(&line.text)
        {
            let number: u32 = c[1].parse().unwrap_or(0);
            let title = c[2].trim().to_string();
            if !out.topics.iter().any(|t| t.number == number) {
                out.topics.push(TopicInfo { number, title });
            } else if let Some(t) = out
                .topics
                .iter_mut()
                .find(|t| t.number == number && t.title.is_empty())
            {
                t.title = title;
            }
            continue;
        }
        kept.push(line);
    }

    let headers: Vec<(usize, u32, Option<u32>)> = kept
        .iter()
        .enumerate()
        .filter_map(|(i, l)| header(&l.text).map(|(n, t)| (i, n, t)))
        .collect();

    let preamble_end = headers.first().map(|h| h.0).unwrap_or(kept.len());
    let preamble: Vec<&str> = kept[..preamble_end]
        .iter()
        .take(4)
        .map(|l| l.text.as_str())
        .collect();
    if !preamble.is_empty() {
        out.title = Some(preamble.join(" "));
        out.exam_code = preamble
            .iter()
            .find_map(|t| EXAM_CODE.captures(t).map(|c| c[1].to_string()));
    }

    let mut current_topic = 1;
    for (h, &(start, number, topic)) in headers.iter().enumerate() {
        let end = headers.get(h + 1).map(|n| n.0).unwrap_or(kept.len());
        if let Some(t) = topic {
            current_topic = t;
        }
        let block = &kept[start..end];
        let next_top = kept.get(end).map(|l| (l.page, l.top));
        out.questions
            .push(parse_block(number, current_topic, block, next_top, pages));
    }
    if !out.topics.iter().any(|t| t.number == 1)
        && out.questions.iter().any(|q| q.question.topic == 1)
    {
        out.topics.insert(
            0,
            TopicInfo {
                number: 1,
                title: String::new(),
            },
        );
    }
    for q in &out.questions {
        if !out.topics.iter().any(|t| t.number == q.question.topic) {
            out.topics.push(TopicInfo {
                number: q.question.topic,
                title: String::new(),
            });
        }
    }
    out.topics.sort_by_key(|t| t.number);
    out
}

/// Recognises `Question #12 Topic 1`. Returns (number, topic).
fn header(text: &str) -> Option<(u32, Option<u32>)> {
    let c = QUESTION_HEADER.captures(text)?;
    let number: u32 = c[2].parse().ok()?;
    let rest = c[3].trim();
    let topic = TOPIC_IN_HEADER
        .captures(rest)
        .and_then(|t| t[1].parse().ok());
    // "Question 3 of the case study ..." is body text, not a header.
    let has_hash = c.get(1).is_some();
    (has_hash && (rest.is_empty() || topic.is_some() || rest.len() < 3)
        || topic.is_some()
        || rest.is_empty())
    .then_some((number, topic))
}

fn parse_block(
    number: u32,
    topic: u32,
    block: &[&Line],
    next_top: Option<(u32, f32)>,
    pages: &[PageInfo],
) -> ParsedQuestion {
    let mut warnings = Vec::new();
    let header = block[0];
    let body_lines = &block[1..];

    let ca_idx = body_lines
        .iter()
        .position(|l| CORRECT_ANSWER.is_match(&l.text));
    let (q_lines, a_lines) = match ca_idx {
        Some(i) => (&body_lines[..i], &body_lines[i..]),
        None => {
            warnings.push("No \"Correct Answer\" line found".to_string());
            (body_lines, &body_lines[body_lines.len()..])
        }
    };

    // ---- answer section -------------------------------------------------
    let answer = AnswerSection::parse(a_lines);

    // ---- question section -----------------------------------------------
    let mut q: Vec<&Line> = q_lines.to_vec();
    let mut visual: Option<VisualVariant> = None;
    if let Some(pos) = q.iter().take(3).position(|l| TYPE_MARKER.is_match(&l.text)) {
        let marker = q[pos].text.to_lowercase();
        visual = Some(if marker.contains("hot") {
            VisualVariant::Hotspot
        } else if marker.contains("drag") {
            VisualVariant::DragDrop
        } else {
            VisualVariant::Simulation
        });
        q.remove(pos);
    }

    let mut case_study = None;
    if let Some(intro) = q.iter().position(|l| INTRO_INFO.is_match(&l.text))
        && let Some(qpos) = q[intro..]
            .iter()
            .position(|l| BARE_QUESTION.is_match(&l.text))
    {
        let qpos = intro + qpos;
        case_study = Some(join_lines(&q[intro + 1..qpos]));
        q.drain(..=qpos);
    }

    let area_idx = q.iter().position(|l| AREA_MARKER.is_match(&l.text));
    if let Some(i) = area_idx
        && visual.is_none()
        && find_options(&q).is_none()
    {
        let marker = q[i].text.to_lowercase();
        visual = Some(if marker.contains("select and place") {
            VisualVariant::DragDrop
        } else {
            VisualVariant::Hotspot
        });
    }

    let (body, kind) = match visual {
        Some(variant) => {
            let (body_part, area_part) = match area_idx {
                Some(i) => (&q[..i], &q[i + 1..]),
                None => (&q[..], &q[q.len()..]),
            };
            let body = join_lines(body_part);
            let kind = build_visual(variant, &body, area_part, &answer, &mut warnings);
            (body, kind)
        }
        None => match find_options(&q) {
            Some((start, options)) => {
                let body = join_lines(&q[..start]);
                let correct = answer.letters(&options, &mut warnings);
                if let Some(n) = expected_count(&body)
                    && n != correct.len()
                    && !correct.is_empty()
                {
                    warnings.push(format!(
                        "Question asks for {n} answers but the key has {}",
                        correct.len()
                    ));
                }
                (body, QuestionKind::Choice { options, correct })
            }
            None => {
                warnings.push("Could not find answer options; showing as image question".into());
                let body = join_lines(&q);
                (
                    body,
                    QuestionKind::Visual {
                        variant: VisualVariant::Other,
                        answer_text: answer.text_opt(),
                    },
                )
            }
        },
    };

    // ---- regions and exhibit detection --------------------------------------
    let q_end = a_lines.first().map(|l| (l.page, l.top));
    let a_end = answer
        .community_line
        .map(|l| (l.page, l.top))
        .or(next_top)
        .or_else(|| block.last().map(|l| (l.page, l.bottom + 4.0)));
    let question_regions = regions(
        (header.page, header.top),
        q_end.or(a_end).unwrap_or((header.page, header.bottom)),
        pages,
    );
    let answer_regions = match a_lines.first() {
        Some(first) => regions(
            (first.page, first.top),
            a_end.unwrap_or((first.page, first.bottom)),
            pages,
        ),
        None => Vec::new(),
    };

    let has_exhibit = matches!(
        kind,
        QuestionKind::Visual { .. } | QuestionKind::Dropdown { .. }
    ) || EXHIBIT.is_match(&body)
        || has_large_gap(header, q_lines, q_end, pages);

    ParsedQuestion {
        question: Question {
            number,
            topic,
            case_study,
            body,
            kind,
            source_explanation: answer.explanation.clone(),
            references: answer.references.clone(),
            community_votes: answer.votes.clone(),
            has_exhibit,
            images: QuestionImages::default(),
        },
        question_regions,
        answer_regions,
        warnings,
    }
}

struct AnswerSection<'a> {
    /// Text after "Correct Answer:" up to the explanation.
    text: String,
    explanation: Option<String>,
    references: Vec<String>,
    votes: Vec<Vote>,
    community_line: Option<&'a Line>,
}

impl<'a> AnswerSection<'a> {
    fn parse(a_lines: &[&'a Line]) -> Self {
        let mut s = AnswerSection {
            text: String::new(),
            explanation: None,
            references: Vec::new(),
            votes: Vec::new(),
            community_line: None,
        };
        let Some(first) = a_lines.first() else {
            return s;
        };

        #[derive(PartialEq)]
        enum Part {
            Answer,
            Explanation,
            Reference,
            Votes,
        }
        let mut part = Part::Answer;
        let mut answer: Vec<String> = vec![
            CORRECT_ANSWER.captures(&first.text).unwrap()[2]
                .trim()
                .to_string(),
        ];
        let mut explanation: Vec<&Line> = Vec::new();
        let mut first_expl = String::new();
        for &line in &a_lines[1..] {
            if COMMUNITY.is_match(&line.text) {
                part = Part::Votes;
                s.community_line.get_or_insert(line);
            } else if part != Part::Votes
                && let Some(c) = EXPLANATION.captures(&line.text)
            {
                part = Part::Explanation;
                first_expl = c[1].trim().to_string();
                continue;
            } else if part != Part::Votes && REFERENCE.is_match(&line.text) {
                part = Part::Reference;
            }
            match part {
                Part::Answer => answer.push(line.text.clone()),
                Part::Explanation => explanation.push(line),
                Part::Reference => {}
                Part::Votes => {}
            }
            if part == Part::Votes {
                for c in VOTE.captures_iter(&line.text) {
                    s.votes.push(Vote {
                        answer: c[1].to_string(),
                        percent: c[2].parse().unwrap_or(0),
                    });
                }
            }
            if part != Part::Votes {
                for u in URL.find_iter(&line.text) {
                    s.references
                        .push(u.as_str().trim_end_matches(['.', ',', ')']).to_string());
                }
            }
        }
        s.text = answer
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let mut expl = join_lines(&explanation);
        if !first_expl.is_empty() {
            expl = if expl.is_empty() {
                first_expl
            } else {
                format!("{first_expl} {expl}")
            };
        }
        if !expl.trim().is_empty() {
            s.explanation = Some(expl);
        }
        s.references.dedup();
        s
    }

    fn text_opt(&self) -> Option<String> {
        (!self.text.trim().is_empty()).then(|| self.text.clone())
    }

    /// Answer letters such as `["A", "C"]`, validated against the options.
    fn letters(&self, options: &[ChoiceOption], warnings: &mut Vec<String>) -> Vec<String> {
        let mut letters: Vec<String> = ANSWER_LETTERS
            .captures(&self.text)
            .map(|c| {
                c[1].chars()
                    .filter(|ch| ch.is_ascii_uppercase())
                    .map(|ch| ch.to_string())
                    .collect()
            })
            .unwrap_or_default();
        letters.dedup();
        let valid = |l: &String| options.iter().any(|o| &o.key == l);
        if letters.is_empty() || !letters.iter().all(valid) {
            if let Some(top) = self.votes.iter().max_by_key(|v| v.percent) {
                warnings.push(format!(
                    "Answer key unreadable (\"{}\"); using community vote {}",
                    self.text.lines().next().unwrap_or(""),
                    top.answer
                ));
                letters = top.answer.chars().map(|c| c.to_string()).collect();
            } else {
                warnings.push(format!(
                    "Answer key unreadable: \"{}\"",
                    self.text.lines().next().unwrap_or("")
                ));
                letters.retain(valid);
            }
        }
        letters
    }
}

/// Locates the lettered options at the end of the question section.
/// Returns the index of the first option line and the parsed options.
fn find_options(q: &[&Line]) -> Option<(usize, Vec<ChoiceOption>)> {
    let starts: Vec<usize> = q
        .iter()
        .enumerate()
        .filter(|(_, l)| OPTION.captures(&l.text).is_some_and(|c| &c[1] == "A"))
        .map(|(i, _)| i)
        .collect();
    // Prefer the last "A." that starts a proper A, B, C... sequence.
    for &start in starts.iter().rev() {
        let mut options: Vec<(String, Vec<&Line>, String)> = Vec::new();
        let mut ok = true;
        for (i, line) in q[start..].iter().enumerate() {
            let expected = (b'A' + options.len() as u8) as char;
            match OPTION.captures(&line.text) {
                Some(c) if c[1].starts_with(expected) => {
                    options.push((expected.to_string(), Vec::new(), c[2].to_string()));
                }
                _ if i == 0 => {
                    ok = false;
                    break;
                }
                _ => {
                    if let Some(last) = options.last_mut() {
                        last.1.push(line);
                    }
                }
            }
        }
        if ok && options.len() >= 2 {
            let parsed = options
                .into_iter()
                .map(|(key, cont, first)| {
                    let rest = join_lines(&cont);
                    let text = match (first.is_empty(), rest.is_empty()) {
                        (_, true) => first,
                        (true, false) => rest,
                        (false, false) => format!("{first} {rest}"),
                    };
                    ChoiceOption { key, text }
                })
                .collect();
            return Some((start, parsed));
        }
    }
    None
}

fn expected_count(body: &str) -> Option<usize> {
    let c = CHOOSE_N.captures(body)?;
    Some(match c[2].to_lowercase().as_str() {
        "two" | "2" => 2,
        "three" | "3" => 3,
        "four" | "4" => 4,
        _ => 5,
    })
}

fn build_visual(
    variant: VisualVariant,
    body: &str,
    area: &[&Line],
    answer: &AnswerSection,
    warnings: &mut Vec<String>,
) -> QuestionKind {
    // Yes/No statement hotspots can be rebuilt from OCR text.
    if variant == VisualVariant::Hotspot && SELECT_YES.is_match(body) {
        let source = format!(
            "{}\n{}",
            answer.text,
            answer.explanation.clone().unwrap_or_default()
        );
        let mut boxes: Vec<(usize, bool)> = BOX_ANSWER
            .captures_iter(&source)
            .filter_map(|c| Some((c[1].parse().ok()?, c[2].eq_ignore_ascii_case("yes"))))
            .collect();
        boxes.sort_by_key(|b| b.0);
        boxes.dedup_by_key(|b| b.0);
        let statements: Vec<String> = area
            .iter()
            .map(|l| TRAILING_YES_NO.replace(&l.text, "").trim().to_string())
            .filter(|s| !s.is_empty() && !YES_NO_ONLY.is_match(s))
            .collect();
        if !boxes.is_empty() && boxes.len() == statements.len() {
            return QuestionKind::Dropdown {
                groups: statements
                    .into_iter()
                    .zip(boxes)
                    .map(|(label, (_, yes))| DropdownGroup {
                        label,
                        options: vec!["Yes".into(), "No".into()],
                        correct: if yes { 0 } else { 1 },
                    })
                    .collect(),
            };
        }
        if !boxes.is_empty() {
            warnings.push(format!(
                "Found {} Yes/No answers but {} statements; kept as image question",
                boxes.len(),
                statements.len()
            ));
        }
    }
    QuestionKind::Visual {
        variant,
        answer_text: answer.text_opt(),
    }
}

/// Re-flows OCR lines into paragraphs.
pub fn join_lines(lines: &[&Line]) -> String {
    let Some(max_right) = lines.iter().map(|l| l.right).reduce(f32::max) else {
        return String::new();
    };
    let min_left = lines.iter().map(|l| l.left).fold(f32::MAX, f32::min);
    let width = (max_right - min_left).max(1.0);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            let prev = lines[i - 1];
            let gap = line.top - prev.bottom;
            let same_page = prev.page == line.page;
            let ends_sentence = prev.text.ends_with(['.', '?', '!', ':']);
            let short = prev.right < max_right - width * 0.15;
            let bullet = line.text.starts_with(['-', '•', '●', '▪', '*', '·']);
            let paragraph = (same_page && gap > prev.height() * 0.8)
                || prev.text.ends_with(':')
                // ExamTopics headings look like "Case study -" / "Requirements -".
                || prev.text.ends_with(" -")
                || bullet
                || (ends_sentence && short);
            out.push(if paragraph { '\n' } else { ' ' });
        }
        out.push_str(&line.text);
    }
    out
}

/// Splits a (page, y) range into per-page regions.
fn regions(start: (u32, f32), end: (u32, f32), pages: &[PageInfo]) -> Vec<Region> {
    let page_h = |p: u32| pages.get(p as usize).map(|i| i.height).unwrap_or(842.0);
    let (sp, st) = start;
    let (ep, et) = if end.0 < sp || (end.0 == sp && end.1 <= st) {
        (sp, st + 20.0)
    } else {
        end
    };
    let mut out = Vec::new();
    for page in sp..=ep {
        let h = page_h(page);
        let top = if page == sp {
            (st - 4.0).max(0.0)
        } else {
            h * 0.05
        };
        let bottom = if page == ep {
            (et - 1.0).min(h)
        } else {
            h * 0.95
        };
        if bottom - top > 6.0 {
            out.push(Region { page, top, bottom });
        }
    }
    out
}

/// A tall empty band between text lines means a picture sits there.
fn has_large_gap(
    header: &Line,
    q_lines: &[&Line],
    q_end: Option<(u32, f32)>,
    pages: &[PageInfo],
) -> bool {
    let mut all: Vec<(u32, f32, f32, f32)> = std::iter::once(header)
        .chain(q_lines.iter().copied())
        .map(|l| (l.page, l.top, l.bottom, l.height()))
        .collect();
    if let Some((p, t)) = q_end {
        all.push((p, t, t, 0.0));
    }
    let mut heights: Vec<f32> = all.iter().map(|a| a.3).filter(|h| *h > 0.0).collect();
    heights.sort_by(f32::total_cmp);
    let median = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    let threshold = (median * 4.0).max(45.0);
    all.windows(2).any(|w| {
        let (a, b) = (w[0], w[1]);
        if a.0 == b.0 {
            b.1 - a.2 > threshold
        } else {
            // Gap at the bottom of a page or the top of the next one.
            let h = pages.get(a.0 as usize).map(|p| p.height).unwrap_or(842.0);
            h * 0.93 - a.2 > threshold * 1.5 || b.1 - h * 0.07 > threshold * 1.5
        }
    })
}

/// How to group questions into practice sets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlan {
    pub title: String,
    pub topic: u32,
    /// Index range into `ParseOutput::questions`.
    pub start: usize,
    pub end: usize,
}

/// Groups questions by topic, then splits each topic into chunks of
/// `chunk_size` questions (0 = no splitting).
pub fn plan_sets(out: &ParseOutput, chunk_size: usize) -> Vec<SetPlan> {
    let mut plans = Vec::new();
    let qs = &out.questions;
    let mut i = 0;
    while i < qs.len() {
        let topic = qs[i].question.topic;
        let mut j = i;
        while j < qs.len() && qs[j].question.topic == topic {
            j += 1;
        }
        let topic_title = out
            .topics
            .iter()
            .find(|t| t.number == topic)
            .map(|t| t.title.clone())
            .filter(|t| !t.is_empty());
        let base = match topic_title {
            Some(t) => format!("Topic {topic} – {t}"),
            None => format!("Topic {topic}"),
        };
        let size = if chunk_size == 0 { j - i } else { chunk_size };
        let chunks = (j - i).div_ceil(size);
        for c in 0..chunks {
            let start = i + c * size;
            let end = (start + size).min(j);
            let title = if chunks > 1 {
                format!(
                    "{base} · Set {} (Q{}–{})",
                    c + 1,
                    qs[start].question.number,
                    qs[end - 1].question.number
                )
            } else {
                base.clone()
            };
            plans.push(SetPlan {
                title,
                topic,
                start,
                end,
            });
        }
        i = j;
    }
    plans
}
