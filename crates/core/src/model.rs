//! Question model shared by the web app, the parser and the explainer worker.
//!
//! A question is stored as JSON (`questions.data` / `questions.source` in Postgres),
//! so every type here is (de)serializable and must stay backwards compatible.

use serde::{Deserialize, Serialize};

/// One exam question exactly as it appears in the PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// `Question #N` from the PDF.
    pub number: u32,
    /// `Topic N` from the PDF (case studies usually live in topics 2+).
    pub topic: u32,
    /// Shared case-study text ("Introductory Info"), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_study: Option<String>,
    /// Question text. Paragraphs are separated by `\n`.
    pub body: String,
    pub kind: QuestionKind,
    /// "Explanation:" text printed after the answer in the PDF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_explanation: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    /// "Community vote distribution" — useful when the official key is wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub community_votes: Vec<Vote>,
    /// The question contains a picture (exhibit, answer area, ...) that the text
    /// cannot represent, so the page snapshot should be shown by default.
    #[serde(default)]
    pub has_exhibit: bool,
    #[serde(default)]
    pub images: QuestionImages,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vote {
    /// e.g. `"B"` or `"AC"`.
    pub answer: String,
    pub percent: u8,
}

/// Storage paths (bucket `question-images`) of page snapshots.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QuestionImages {
    /// Snapshot of the question area (from the header to "Correct Answer:").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Snapshot of the answer area (after "Correct Answer:").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionKind {
    /// Classic lettered options. One correct key = single choice (radio),
    /// several = multiple choice (checkboxes, "Choose two").
    Choice {
        options: Vec<ChoiceOption>,
        correct: Vec<String>,
    },
    /// HOTSPOT / Yes-No statements: every group is a dropdown with one correct option.
    Dropdown { groups: Vec<DropdownGroup> },
    /// DRAG DROP: drag `items` into `slots`. With `ordered`, slots are steps of a sequence.
    DragDrop {
        items: Vec<String>,
        slots: Vec<DragSlot>,
        #[serde(default)]
        ordered: bool,
    },
    /// Image-based question that has not been turned into an interactive one yet.
    /// The user looks at the snapshots, reveals the answer snapshot and grades themselves.
    Visual {
        variant: VisualVariant,
        /// OCR text found inside the answer area ("Box 1: Yes", ...).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropdownGroup {
    /// Statement / box label shown next to the dropdown.
    pub label: String,
    pub options: Vec<String>,
    /// Index into `options`.
    pub correct: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DragSlot {
    pub label: String,
    /// Index into `items`.
    pub correct: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualVariant {
    Hotspot,
    DragDrop,
    Simulation,
    Other,
}

impl VisualVariant {
    pub fn label(self) -> &'static str {
        match self {
            VisualVariant::Hotspot => "Hotspot",
            VisualVariant::DragDrop => "Drag & drop",
            VisualVariant::Simulation => "Simulation",
            VisualVariant::Other => "Image",
        }
    }
}

impl QuestionKind {
    pub fn label(&self) -> &'static str {
        match self {
            QuestionKind::Choice { correct, .. } if correct.len() > 1 => "Multiple choice",
            QuestionKind::Choice { .. } => "Single choice",
            QuestionKind::Dropdown { .. } => "Hotspot",
            QuestionKind::DragDrop { ordered: true, .. } => "Ordering",
            QuestionKind::DragDrop { .. } => "Drag & drop",
            QuestionKind::Visual { variant, .. } => variant.label(),
        }
    }

    /// Short, human readable form of the correct answer.
    pub fn answer_summary(&self) -> String {
        match self {
            QuestionKind::Choice { correct, .. } => correct.join(""),
            QuestionKind::Dropdown { groups } => groups
                .iter()
                .map(|g| g.options.get(g.correct).cloned().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" / "),
            QuestionKind::DragDrop { items, slots, .. } => slots
                .iter()
                .map(|s| items.get(s.correct).cloned().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" → "),
            QuestionKind::Visual { answer_text, .. } => answer_text
                .clone()
                .unwrap_or_else(|| "See answer image".into()),
        }
    }
}

/// What a user answered. Stored in `question_progress.response`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Response {
    Choice(Vec<String>),
    Dropdown(Vec<Option<usize>>),
    DragDrop(Vec<Option<usize>>),
    /// Self assessment for [`QuestionKind::Visual`].
    SelfGraded(bool),
}

impl Response {
    pub fn empty_for(kind: &QuestionKind) -> Response {
        match kind {
            QuestionKind::Choice { .. } => Response::Choice(Vec::new()),
            QuestionKind::Dropdown { groups } => Response::Dropdown(vec![None; groups.len()]),
            QuestionKind::DragDrop { slots, .. } => Response::DragDrop(vec![None; slots.len()]),
            QuestionKind::Visual { .. } => Response::SelfGraded(false),
        }
    }

    /// Whether the user has filled in enough to check the answer.
    pub fn is_complete(&self, kind: &QuestionKind) -> bool {
        match (self, kind) {
            (Response::Choice(sel), QuestionKind::Choice { correct, .. }) => {
                !sel.is_empty() && sel.len() >= correct.len().max(1)
            }
            (Response::Dropdown(sel), _) | (Response::DragDrop(sel), _) => {
                sel.iter().all(Option::is_some)
            }
            (Response::SelfGraded(_), _) => true,
            _ => false,
        }
    }
}

/// Grades a response against the question's current answer key.
pub fn grade(kind: &QuestionKind, response: &Response) -> bool {
    match (kind, response) {
        (QuestionKind::Choice { correct, .. }, Response::Choice(selected)) => {
            let mut a: Vec<&str> = correct.iter().map(String::as_str).collect();
            let mut b: Vec<&str> = selected.iter().map(String::as_str).collect();
            a.sort_unstable();
            a.dedup();
            b.sort_unstable();
            b.dedup();
            a == b
        }
        (QuestionKind::Dropdown { groups }, Response::Dropdown(selected)) => {
            groups.len() == selected.len()
                && groups
                    .iter()
                    .zip(selected)
                    .all(|(g, s)| *s == Some(g.correct))
        }
        (QuestionKind::DragDrop { items, slots, .. }, Response::DragDrop(selected)) => {
            slots.len() == selected.len()
                && slots.iter().zip(selected).all(|(slot, s)| match s {
                    // Items with identical text are interchangeable.
                    Some(i) => items.get(*i) == items.get(slot.correct),
                    None => false,
                })
        }
        (QuestionKind::Visual { .. }, Response::SelfGraded(ok)) => *ok,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(correct: &[&str]) -> QuestionKind {
        QuestionKind::Choice {
            options: ["A", "B", "C", "D"]
                .iter()
                .map(|k| ChoiceOption {
                    key: k.to_string(),
                    text: format!("opt {k}"),
                })
                .collect(),
            correct: correct.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn grades_choice_ignoring_order() {
        let kind = choice(&["A", "C"]);
        assert!(grade(
            &kind,
            &Response::Choice(vec!["C".into(), "A".into()])
        ));
        assert!(!grade(&kind, &Response::Choice(vec!["A".into()])));
        assert!(!grade(
            &kind,
            &Response::Choice(vec!["A".into(), "B".into(), "C".into()])
        ));
    }

    #[test]
    fn grades_dropdown_and_dragdrop() {
        let kind = QuestionKind::Dropdown {
            groups: vec![
                DropdownGroup {
                    label: "x".into(),
                    options: vec!["Yes".into(), "No".into()],
                    correct: 1,
                },
                DropdownGroup {
                    label: "y".into(),
                    options: vec!["Yes".into(), "No".into()],
                    correct: 0,
                },
            ],
        };
        assert!(grade(&kind, &Response::Dropdown(vec![Some(1), Some(0)])));
        assert!(!grade(&kind, &Response::Dropdown(vec![Some(1), None])));

        let kind = QuestionKind::DragDrop {
            items: vec!["a".into(), "b".into(), "a".into()],
            slots: vec![
                DragSlot {
                    label: "1".into(),
                    correct: 0,
                },
                DragSlot {
                    label: "2".into(),
                    correct: 1,
                },
            ],
            ordered: true,
        };
        assert!(grade(&kind, &Response::DragDrop(vec![Some(2), Some(1)])));
        assert!(!grade(&kind, &Response::DragDrop(vec![Some(1), Some(0)])));
    }

    #[test]
    fn serde_round_trip() {
        let q = Question {
            number: 1,
            topic: 1,
            case_study: None,
            body: "Body".into(),
            kind: choice(&["B"]),
            source_explanation: None,
            references: vec![],
            community_votes: vec![Vote {
                answer: "B".into(),
                percent: 90,
            }],
            has_exhibit: false,
            images: QuestionImages::default(),
        };
        let json = serde_json::to_string(&q).unwrap();
        assert!(json.contains(r#""type":"choice""#));
        assert_eq!(serde_json::from_str::<Question>(&json).unwrap(), q);
        let r = Response::Dropdown(vec![Some(1), None]);
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"type":"dropdown","value":[1,null]}"#
        );
    }
}

/// Claude's explanation of a question. Stored in `explanations.content`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    /// Why the correct answer is correct, in a short paragraph.
    pub summary: String,
    /// The answer Claude believes is correct, e.g. `"B"`, `"AC"` or `"Yes / No / Yes"`.
    pub correct_answer: String,
    /// Whether Claude agrees with the answer key printed in the PDF.
    pub agrees_with_key: bool,
    pub confidence: Confidence,
    /// One entry per option / dropdown / slot, explaining why it is right or wrong.
    pub options: Vec<OptionExplanation>,
    #[serde(default)]
    pub key_concepts: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionExplanation {
    /// Option key ("A") or label of the dropdown / slot.
    pub key: String,
    pub correct: bool,
    pub reason: String,
}
