//! Prompt + JSON schema for explaining a question, and conversion of Claude's answer
//! into `examcore` types.

use anyhow::{Result, bail};
use examcore::{DragSlot, DropdownGroup, Explanation, Question, QuestionKind};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write;

pub struct PromptInput<'a> {
    pub question: &'a Question,
    /// The answer key exactly as printed in the PDF.
    pub source: &'a Question,
    pub cert: Option<(&'a str, &'a str)>,
    pub question_image: Option<&'a str>,
    pub answer_image: Option<&'a str>,
}

pub fn is_visual(q: &Question) -> bool {
    matches!(q.kind, QuestionKind::Visual { .. })
}

pub fn build(input: &PromptInput) -> String {
    let q = input.question;
    let mut p = String::new();
    match input.cert {
        Some((code, name)) => {
            let _ = writeln!(
                p,
                "You are an expert instructor for the {code} ({name}) certification exam."
            );
        }
        None => p.push_str("You are an expert certification exam instructor.\n"),
    }
    p.push_str(
        "Explain the practice question below, taken from an ExamTopics question dump. \
         The dump's answer key is sometimes wrong: work out the correct answer yourself from current \
         official documentation and best practice, then compare it with the key.\n\n",
    );
    let _ = writeln!(
        p,
        "## Question #{} (Topic {}) — {}",
        q.number,
        q.topic,
        q.kind.label()
    );
    if let Some(cs) = &q.case_study {
        let _ = writeln!(p, "\n### Case study\n{cs}");
    }
    let _ = writeln!(p, "\n### Question\n{}", q.body);

    match &q.kind {
        QuestionKind::Choice { options, .. } => {
            p.push_str("\n### Options\n");
            for o in options {
                let _ = writeln!(p, "{}. {}", o.key, o.text);
            }
        }
        QuestionKind::Dropdown { groups } => {
            p.push_str("\n### Answer area (one dropdown per line)\n");
            for g in groups {
                let _ = writeln!(p, "- {} → [{}]", g.label, g.options.join(" | "));
            }
        }
        QuestionKind::DragDrop {
            items,
            slots,
            ordered,
        } => {
            let _ = writeln!(p, "\n### Items\n{}", items.join("\n"));
            let _ = writeln!(
                p,
                "\n### {} \n{}",
                if *ordered {
                    "Steps (in order)"
                } else {
                    "Targets"
                },
                slots
                    .iter()
                    .map(|s| s.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        QuestionKind::Visual { .. } => {}
    }

    let _ = writeln!(
        p,
        "\n### Answer key in the dump\n{}",
        input.source.kind.answer_summary()
    );
    if q.kind != input.source.kind {
        let _ = writeln!(
            p,
            "(An editor has since changed the key to: {})",
            q.kind.answer_summary()
        );
    }
    if !q.community_votes.is_empty() {
        let votes: Vec<String> = q
            .community_votes
            .iter()
            .map(|v| format!("{} {}%", v.answer, v.percent))
            .collect();
        let _ = writeln!(p, "\n### Community vote distribution\n{}", votes.join(", "));
    }
    if let Some(e) = &q.source_explanation {
        let _ = writeln!(
            p,
            "\n### Explanation printed in the dump (may be wrong)\n{e}"
        );
    }

    if input.question_image.is_some() || input.answer_image.is_some() {
        p.push_str("\n### Images\nThe question contains pictures that the text above cannot show. Read these files first:\n");
        if let Some(f) = input.question_image {
            let _ = writeln!(
                p,
                "- `{f}`: the question as printed (exhibits, hot area, items)"
            );
        }
        if let Some(f) = input.answer_image {
            let _ = writeln!(
                p,
                "- `{f}`: the answer area as printed in the dump (its answer key)"
            );
        }
    }

    p.push_str(
        "\n### How to answer\n\
         - `summary`: 2–5 sentences explaining why the correct answer is correct, written for someone studying for the exam.\n\
         - `options`: one entry per answer option (key = the option letter), per dropdown (key = short label of the statement/box) \
         or per drag & drop target/step (key = target label). Say why each is correct or wrong.\n\
         - `correct_answer`: the answer you believe is correct — option letters such as `B` or `AC` for lettered options, \
         otherwise a short text such as `Yes / No / Yes`.\n\
         - `agrees_with_key`: whether your answer matches the dump's answer key.\n\
         - `confidence`: how sure you are.\n\
         - `key_concepts`: 2–5 short topic tags.\n\
         - `references`: official documentation URLs, only ones you are confident exist (may be empty).\n",
    );
    if is_visual(q) {
        p.push_str(
            "- `interactive`: transcribe the image-based question so it can be answered interactively, using the exact wording \
             from the image. Use `dropdown` for hotspot answer areas (each dropdown/Yes-No statement becomes a group with its \
             options and the index of the correct one according to the dump's answer key / answer image), `drag_drop` for \
             drag & drop (all source items, every target or step with the index of its correct item per the dump's key; \
             `ordered` = true when the order matters), or `none` if it cannot be represented that way (e.g. clicking on an area of a screenshot). \
             If you disagree with the key, keep the transcription faithful and say so through `agrees_with_key` and \
             `correct_answer`.\n",
        );
    }
    p.push_str("\nReply only through the structured output.");
    p
}

pub fn schema(visual: bool) -> Value {
    let mut properties = json!({
        "summary": { "type": "string" },
        "correct_answer": { "type": "string" },
        "agrees_with_key": { "type": "boolean" },
        "confidence": { "type": "string", "enum": ["high", "medium", "low"] },
        "options": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "correct": { "type": "boolean" },
                    "reason": { "type": "string" }
                },
                "required": ["key", "correct", "reason"]
            }
        },
        "key_concepts": { "type": "array", "items": { "type": "string" } },
        "references": { "type": "array", "items": { "type": "string" } }
    });
    let mut required = vec![
        "summary",
        "correct_answer",
        "agrees_with_key",
        "confidence",
        "options",
        "key_concepts",
        "references",
    ];
    if visual {
        properties["interactive"] = json!({
            "type": "object",
            "properties": {
                "type": { "type": "string", "enum": ["dropdown", "drag_drop", "none"] },
                "groups": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "options": { "type": "array", "items": { "type": "string" } },
                            "correct_index": { "type": "integer" }
                        },
                        "required": ["label", "options", "correct_index"]
                    }
                },
                "items": { "type": "array", "items": { "type": "string" } },
                "slots": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "correct_item_index": { "type": "integer" }
                        },
                        "required": ["label", "correct_item_index"]
                    }
                },
                "ordered": { "type": "boolean" }
            },
            "required": ["type"]
        });
        required.push("interactive");
    }
    json!({ "type": "object", "properties": properties, "required": required })
}

#[derive(Deserialize)]
struct Interactive {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    slots: Vec<Slot>,
    #[serde(default)]
    ordered: bool,
}

#[derive(Deserialize)]
struct Group {
    label: String,
    options: Vec<String>,
    correct_index: i64,
}

#[derive(Deserialize)]
struct Slot {
    label: String,
    correct_item_index: i64,
}

/// Splits Claude's structured output into the explanation and an optional new question kind.
pub fn parse(output: &Value) -> Result<(Explanation, Option<QuestionKind>)> {
    let mut value = output.clone();
    let interactive = value.as_object_mut().and_then(|o| o.remove("interactive"));
    let explanation: Explanation = serde_json::from_value(value)?;
    if explanation.summary.trim().is_empty() {
        bail!("empty explanation");
    }
    let kind = match interactive {
        Some(v) => convert(serde_json::from_value(v)?),
        None => None,
    };
    Ok((explanation, kind))
}

fn convert(i: Interactive) -> Option<QuestionKind> {
    match i.kind.as_str() {
        "dropdown" => {
            let groups: Vec<DropdownGroup> = i
                .groups
                .into_iter()
                .filter(|g| {
                    g.options.len() >= 2
                        && g.correct_index >= 0
                        && (g.correct_index as usize) < g.options.len()
                })
                .map(|g| DropdownGroup {
                    label: g.label,
                    options: g.options,
                    correct: g.correct_index as usize,
                })
                .collect();
            (!groups.is_empty()).then_some(QuestionKind::Dropdown { groups })
        }
        "drag_drop" => {
            let n = i.items.len();
            let slots: Vec<DragSlot> = i
                .slots
                .into_iter()
                .filter(|s| s.correct_item_index >= 0 && (s.correct_item_index as usize) < n)
                .map(|s| DragSlot {
                    label: s.label,
                    correct: s.correct_item_index as usize,
                })
                .collect();
            (n >= 2 && !slots.is_empty()).then_some(QuestionKind::DragDrop {
                items: i.items,
                slots,
                ordered: i.ordered,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_interactive_dropdown_and_rejects_bad_indices() {
        let out = json!({
            "summary": "Because.",
            "correct_answer": "Yes / No",
            "agrees_with_key": true,
            "confidence": "high",
            "options": [{ "key": "Box 1", "correct": true, "reason": "r" }],
            "key_concepts": [],
            "references": [],
            "interactive": {
                "type": "dropdown",
                "groups": [
                    { "label": "VM1 can reach VM2", "options": ["Yes", "No"], "correct_index": 0 },
                    { "label": "bad", "options": ["Yes", "No"], "correct_index": 5 }
                ]
            }
        });
        let (exp, kind) = parse(&out).unwrap();
        assert_eq!(exp.correct_answer, "Yes / No");
        match kind {
            Some(QuestionKind::Dropdown { groups }) => assert_eq!(groups.len(), 1),
            k => panic!("unexpected {k:?}"),
        }
    }

    #[test]
    fn schema_only_asks_for_interactive_on_visual_questions() {
        assert!(schema(false)["properties"].get("interactive").is_none());
        assert!(
            schema(true)["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "interactive")
        );
    }
}
