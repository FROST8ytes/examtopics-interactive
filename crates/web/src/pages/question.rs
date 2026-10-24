//! Interactive renderers for every question kind, plus the explanation panel.

use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::select::{Select, SelectOption};
use crate::data::{ExplanationRow, JobRow, QuestionRow};
use crate::ui::{Banner, Spinner};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    Check, Eye, EyeOff, RefreshCw, Sparkles, ThumbsDown, ThumbsUp, Wand, X,
};
use examcore::{
    ChoiceOption, Confidence, DragSlot, DropdownGroup, Question, QuestionKind, Response, Vote,
};
use std::collections::HashMap;

/// Renders the question text, snapshots and the answer input for its kind.
#[component]
pub fn QuestionView(
    row: QuestionRow,
    images: HashMap<String, String>,
    response: Response,
    checked: bool,
    on_change: EventHandler<Response>,
    /// Visual questions grade themselves through the self-assessment buttons.
    on_self_grade: EventHandler<bool>,
) -> Element {
    let q = row.data.clone();
    let mut show_original = use_signal(|| false);
    let question_img = q
        .images
        .question
        .as_ref()
        .and_then(|p| images.get(p))
        .cloned();
    let answer_img = q
        .images
        .answer
        .as_ref()
        .and_then(|p| images.get(p))
        .cloned();
    let is_visual = matches!(q.kind, QuestionKind::Visual { .. });
    let show_question_img = q.has_exhibit || is_visual || show_original();

    rsx! {
        div { class: "stack",
            if let Some(cs) = &q.case_study {
                details { class: "case-study",
                    summary { "Case study (click to expand)" }
                    div { class: "content", "{cs}" }
                }
            }
            div { class: "q-body",
                for para in q.body.split('\n').filter(|p| !p.trim().is_empty()) {
                    p { "{para}" }
                }
            }
            if show_question_img {
                if let Some(url) = &question_img {
                    div { class: "snapshot",
                        img { src: "{url}", alt: "Question {q.number} as printed in the PDF", loading: "lazy" }
                        div { class: "snapshot-caption", "Original page" }
                    }
                } else if is_visual || q.has_exhibit {
                    Banner { kind: "warn",
                        span { class: "small", "This question has an image that wasn't captured. Re-import the PDF with page snapshots enabled." }
                    }
                }
            }
            match &q.kind {
                QuestionKind::Choice { options, correct } => rsx! {
                    ChoiceInput {
                        options: options.clone(),
                        correct: correct.clone(),
                        votes: q.community_votes.clone(),
                        response: response.clone(),
                        checked,
                        on_change,
                    }
                },
                QuestionKind::Dropdown { groups } => rsx! {
                    DropdownInput { key: "{row.id}-{checked}", groups: groups.clone(), response: response.clone(), checked, on_change }
                },
                QuestionKind::DragDrop { items, slots, ordered } => rsx! {
                    DragDropInput {
                        key: "{row.id}-{checked}",
                        items: items.clone(),
                        slots: slots.clone(),
                        ordered: *ordered,
                        response: response.clone(),
                        checked,
                        on_change,
                    }
                },
                QuestionKind::Visual { answer_text, .. } => rsx! {
                    VisualInput {
                        key: "{row.id}-{checked}",
                        answer_img: answer_img.clone(),
                        answer_text: answer_text.clone(),
                        response: response.clone(),
                        checked,
                        on_self_grade,
                    }
                },
            }
            if !is_visual && question_img.is_some() && !q.has_exhibit {
                div {
                    button { class: "link small", onclick: move |_| show_original.toggle(),
                        span { class: "row", style: "gap:.35rem",
                            if show_original() { EyeOff { size: "14px" } "Hide original page" } else { Eye { size: "14px" } "Compare with the original page" }
                        }
                    }
                }
            }
        }
    }
}

// ---- choice --------------------------------------------------------------------

#[component]
fn ChoiceInput(
    options: Vec<ChoiceOption>,
    correct: Vec<String>,
    votes: Vec<Vote>,
    response: Response,
    checked: bool,
    on_change: EventHandler<Response>,
) -> Element {
    let multi = correct.len() > 1;
    let selected = match &response {
        Response::Choice(v) => v.clone(),
        _ => Vec::new(),
    };
    let vote_share = |key: &str| -> u32 {
        votes
            .iter()
            .filter(|v| v.answer.contains(key))
            .map(|v| v.percent as u32)
            .sum()
    };

    rsx! {
        div { class: "stack-sm",
            if multi && !checked {
                span { class: "small muted", "Select {correct.len()} answers." }
            }
            div { class: "options", role: if multi { "group" } else { "radiogroup" },
                for opt in options.iter() {
                    {
                        let key = opt.key.clone();
                        let is_sel = selected.contains(&key);
                        let is_ok = correct.contains(&key);
                        let state = match (checked, is_ok, is_sel) {
                            (false, _, true) => "selected",
                            (true, true, true) => "correct",
                            (true, false, true) => "incorrect",
                            (true, true, false) => "missed",
                            _ => "",
                        };
                        let share = vote_share(&key);
                        let selected = selected.clone();
                        rsx! {
                            button {
                                key: "{key}",
                                class: "option",
                                "data-state": state,
                                "data-multi": "{multi}",
                                role: if multi { "checkbox" } else { "radio" },
                                aria_checked: "{is_sel}",
                                disabled: checked,
                                onclick: move |_| {
                                    let mut next = if multi { selected.clone() } else { Vec::new() };
                                    if multi && is_sel {
                                        next.retain(|k| k != &key);
                                    } else if !next.contains(&key) {
                                        next.push(key.clone());
                                    }
                                    next.sort();
                                    on_change.call(Response::Choice(next));
                                },
                                span { class: "key", "{opt.key}" }
                                span { class: "text", "{opt.text}" }
                                if checked {
                                    span { class: "mark row", style: "gap:.4rem",
                                        if share > 0 { span { class: "vote", "{share}% voted" } }
                                        if is_ok { Check { size: "18px", stroke: "var(--ok)" } }
                                        else if is_sel { X { size: "18px", stroke: "var(--bad)" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---- hotspot dropdowns ---------------------------------------------------------------

#[component]
fn DropdownInput(
    groups: Vec<DropdownGroup>,
    response: Response,
    checked: bool,
    on_change: EventHandler<Response>,
) -> Element {
    let selected = match &response {
        Response::Dropdown(v) if v.len() == groups.len() => v.clone(),
        _ => vec![None; groups.len()],
    };
    rsx! {
        div { class: "hotspot",
            span { class: "small muted", "Answer Area — choose the correct option in each dropdown." }
            for (gi, group) in groups.iter().enumerate() {
                {
                    let current = selected[gi];
                    let state = match (checked, current == Some(group.correct)) {
                        (false, _) => "",
                        (true, true) => "correct",
                        (true, false) => "incorrect",
                    };
                    let selected = selected.clone();
                    rsx! {
                        div { key: "{gi}", class: "hotspot-row", "data-state": state,
                            span { "{group.label}" }
                            Select::<usize> {
                                default_value: current,
                                disabled: checked,
                                on_value_change: move |v: Option<usize>| {
                                    let mut next = selected.clone();
                                    next[gi] = v;
                                    on_change.call(Response::Dropdown(next));
                                },
                                for (oi, opt) in group.options.iter().enumerate() {
                                    SelectOption::<usize> { key: "{oi}", value: oi, index: oi, text_value: opt.clone(), "{opt}" }
                                }
                            }
                            if checked && current != Some(group.correct) {
                                span { class: "answer-note",
                                    "Correct: {group.options.get(group.correct).cloned().unwrap_or_default()}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---- drag & drop ----------------------------------------------------------------------

#[component]
fn DragDropInput(
    items: Vec<String>,
    slots: Vec<DragSlot>,
    ordered: bool,
    response: Response,
    checked: bool,
    on_change: EventHandler<Response>,
) -> Element {
    let mut picked = use_signal(|| None::<usize>);
    let mut over = use_signal(|| None::<usize>);
    let placed = match &response {
        Response::DragDrop(v) if v.len() == slots.len() => v.clone(),
        _ => vec![None; slots.len()],
    };

    let place = {
        let placed = placed.clone();
        move |slot: usize, item: Option<usize>| {
            let mut next = placed.clone();
            next[slot] = item;
            on_change.call(Response::DragDrop(next));
        }
    };

    rsx! {
        div { class: "stack-sm",
            span { class: "small muted",
                if ordered { "Drag the actions into the answer area in the correct order (or click an action, then a slot)." }
                else { "Drag each item onto its target (or click an item, then a target). Items can be used more than once." }
            }
            div { class: "dnd",
                div { class: "dnd-col",
                    h4 { if ordered { "Actions" } else { "Items" } }
                    for (i, item) in items.iter().enumerate() {
                        button {
                            key: "{i}",
                            class: "chip",
                            draggable: !checked,
                            disabled: checked,
                            "data-selected": "{picked() == Some(i)}",
                            "data-used": "{placed.contains(&Some(i))}",
                            ondragstart: move |_| picked.set(Some(i)),
                            onclick: move |_| picked.set(if picked() == Some(i) { None } else { Some(i) }),
                            "{item}"
                        }
                    }
                }
                div { class: "dnd-col",
                    h4 { "Answer area" }
                    for (si, slot) in slots.iter().enumerate() {
                        {
                            let current = placed[si];
                            let is_ok = current.and_then(|c| items.get(c)) == items.get(slot.correct);
                            let state = if !checked { "" } else if is_ok { "correct" } else { "incorrect" };
                            let place_drop = place.clone();
                            let place_click = place.clone();
                            rsx! {
                                div { key: "{si}", class: "slot",
                                    span { class: "slot-label", "{slot.label}" }
                                    div {
                                        class: "slot-drop",
                                        "data-filled": "{current.is_some()}",
                                        "data-over": "{over() == Some(si)}",
                                        "data-state": state,
                                        ondragover: move |e| { e.prevent_default(); over.set(Some(si)); },
                                        ondragleave: move |_| over.set(None),
                                        ondrop: move |e| {
                                            e.prevent_default();
                                            over.set(None);
                                            if let Some(item) = picked() {
                                                place_drop(si, Some(item));
                                                picked.set(None);
                                            }
                                        },
                                        onclick: move |_| {
                                            if checked { return; }
                                            match picked() {
                                                Some(item) => { place_click(si, Some(item)); picked.set(None); }
                                                None => place_click(si, None),
                                            }
                                        },
                                        match current.and_then(|c| items.get(c)) {
                                            Some(text) => rsx! {
                                                span { "{text}" }
                                                if !checked { X { size: "14px" } }
                                            },
                                            None => rsx! { span { class: "muted small", "Drop here" } },
                                        }
                                    }
                                    if checked && !is_ok {
                                        span { class: "tiny", style: "color:var(--ok);font-weight:600",
                                            "Correct: {items.get(slot.correct).cloned().unwrap_or_default()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---- image-only questions ------------------------------------------------------------------

#[component]
fn VisualInput(
    answer_img: Option<String>,
    answer_text: Option<String>,
    response: Response,
    checked: bool,
    on_self_grade: EventHandler<bool>,
) -> Element {
    let mut revealed = use_signal(|| checked);
    let shown = revealed() || checked;
    rsx! {
        div { class: "stack",
            Banner { kind: "info",
                span { class: "small",
                    "This is an image-based question. Work out your answer from the picture above, then reveal the official answer and grade yourself. "
                    "Once Claude has processed it, it becomes fully interactive."
                }
            }
            if !shown {
                div { Button { variant: ButtonVariant::Secondary, onclick: move |_| revealed.set(true), Eye { size: "16px" } "Reveal answer" } }
            } else {
                div { class: "stack-sm",
                    h3 { "Correct answer" }
                    if let Some(url) = &answer_img {
                        div { class: "snapshot",
                            img { src: "{url}", alt: "Correct answer", loading: "lazy" }
                        }
                    }
                    if let Some(text) = &answer_text {
                        p { class: "pre-wrap small", "{text}" }
                    }
                    if answer_img.is_none() && answer_text.is_none() {
                        p { class: "muted small", "No answer was found in the PDF for this question." }
                    }
                }
                if !checked {
                    div { class: "row",
                        span { class: "small bold", "Did you get it right?" }
                        Button { variant: ButtonVariant::Outline, onclick: move |_| on_self_grade.call(true), ThumbsUp { size: "16px" } "Yes" }
                        Button { variant: ButtonVariant::Outline, onclick: move |_| on_self_grade.call(false), ThumbsDown { size: "16px" } "No" }
                    }
                } else if let Response::SelfGraded(ok) = response {
                    span { class: "small muted", if ok { "You marked this as correct." } else { "You marked this as incorrect." } }
                }
            }
        }
    }
}

// ---- answer key details -------------------------------------------------------------------

/// "ExamTopics key / community vote / edited" line shown after answering.
#[component]
pub fn AnswerKeyInfo(row: QuestionRow) -> Element {
    let source_key = row.source.kind.answer_summary();
    let current_key = row.data.kind.answer_summary();
    let votes = &row.data.community_votes;
    rsx! {
        div { class: "row small", style: "gap:.5rem",
            span { class: "muted", "ExamTopics answer:" }
            Badge { variant: BadgeVariant::Outline, "{short(&source_key)}" }
            if row.key_edited() && current_key != source_key {
                span { class: "muted", "· corrected to" }
                Badge { variant: BadgeVariant::Primary, "{short(&current_key)}" }
            }
            if !votes.is_empty() {
                span { class: "muted", "· Community:" }
                for v in votes.iter().take(4) {
                    Badge { variant: BadgeVariant::Secondary, "{v.answer} {v.percent}%" }
                }
            }
        }
    }
}

fn short(s: &str) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

// ---- Claude explanation ------------------------------------------------------------------------

#[component]
pub fn ExplanationPanel(
    row: QuestionRow,
    explanation: Option<ExplanationRow>,
    job: Option<JobRow>,
    can_edit: bool,
    /// On the explainer allowlist; everyone else is pointed at their own Claude.
    can_request: bool,
    on_request: EventHandler<bool>,
    on_apply_answer: EventHandler<String>,
) -> Element {
    let header = rsx! {
        div { class: "explain-head",
            Sparkles { class: "claude-mark", size: "18px" }
            h3 { "Claude's explanation" }
        }
    };

    let Some(exp) = explanation else {
        if !can_request {
            return rsx! {
                div { class: "explain",
                    {header}
                    OwnClaude { row: row.clone() }
                }
            };
        }
        let status = job.as_ref().map(|j| j.status.as_str());
        return rsx! {
            div { class: "explain",
                {header}
                match status {
                    Some("pending") | Some("processing") => rsx! {
                        div { class: "row small muted pulse",
                            Spinner { size: "15px" }
                            if status == Some("processing") { "Claude is writing an explanation for this question…" }
                            else { "Queued — the explainer will pick this up shortly. You can keep going; it will appear here." }
                        }
                    },
                    Some("failed") => rsx! {
                        div { class: "stack-sm",
                            span { class: "error-text", "Claude couldn't explain this question: {job.as_ref().and_then(|j| j.error.clone()).unwrap_or_default()}" }
                            div { Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, onclick: move |_| on_request.call(true), RefreshCw { size: "14px" } "Try again" } }
                        }
                    },
                    _ => rsx! {
                        div { class: "row",
                            span { class: "small muted", "No explanation yet." }
                            Button { size: ButtonSize::Sm, onclick: move |_| on_request.call(false), Sparkles { size: "14px" } "Ask Claude" }
                        }
                    },
                }
            }
        };
    };

    let c = exp.content.clone();
    let is_choice = matches!(row.data.kind, QuestionKind::Choice { .. });
    let suggested = c.correct_answer.clone();
    let differs_from_current = is_choice
        && normalize_letters(&suggested) != normalize_letters(&row.data.kind.answer_summary());
    rsx! {
        div { class: "explain",
            {header}
            div { class: "row", style: "gap:.4rem",
                if c.agrees_with_key {
                    Badge { variant: BadgeVariant::Secondary, "Agrees with the ExamTopics answer" }
                } else {
                    Badge { variant: BadgeVariant::Destructive, "Disagrees — Claude's answer: {short(&suggested)}" }
                }
                Badge { variant: BadgeVariant::Outline,
                    match c.confidence { Confidence::High => "High confidence", Confidence::Medium => "Medium confidence", Confidence::Low => "Low confidence" }
                }
            }
            if !c.agrees_with_key && can_edit && differs_from_current && !normalize_letters(&suggested).is_empty() {
                Banner { kind: "warn",
                    div { class: "row-between full",
                        span { class: "small", "If Claude is right, fix the answer key for everyone." }
                        Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, onclick: move |_| on_apply_answer.call(suggested.clone()),
                            Wand { size: "14px" } "Use Claude's answer"
                        }
                    }
                }
            }
            p { class: "pre-wrap", "{c.summary}" }
            if !c.options.is_empty() {
                ul { class: "explain-list",
                    for o in c.options.iter() {
                        li { key: "{o.key}",
                            span { class: "k", "data-ok": "{o.correct}", "{o.key}" }
                            span { "{o.reason}" }
                        }
                    }
                }
            }
            if !c.key_concepts.is_empty() {
                div { class: "concepts",
                    for k in c.key_concepts.iter() { Badge { variant: BadgeVariant::Outline, "{k}" } }
                }
            }
            if !c.references.is_empty() {
                div { class: "refs",
                    for r in c.references.iter() { a { href: "{r}", target: "_blank", rel: "noopener noreferrer", "{r}" } }
                }
            }
            div { class: "row-between tiny muted",
                span { "Generated by {exp.model.clone().unwrap_or_else(|| \"Claude\".into())} · AI explanations can be wrong — check the references." }
                if can_edit {
                    button { class: "link tiny", onclick: move |_| on_request.call(true), "Regenerate" }
                }
            }
        }
    }
}

fn normalize_letters(s: &str) -> String {
    let mut v: Vec<char> = s.chars().filter(|c| c.is_ascii_uppercase()).collect();
    v.sort_unstable();
    v.dedup();
    v.into_iter().collect()
}

/// Text from the PDF's own "Explanation:" section and references.
#[component]
pub fn SourceExplanation(q: Question) -> Element {
    if q.source_explanation.is_none() && q.references.is_empty() {
        return rsx! {};
    }
    rsx! {
        details { class: "case-study",
            summary { "Explanation from the PDF" }
            div { class: "content stack-sm",
                if let Some(e) = &q.source_explanation { p { "{e}" } }
                if !q.references.is_empty() {
                    div { class: "refs",
                        for r in q.references.iter() { a { href: "{r}", target: "_blank", rel: "noopener noreferrer", "{r}" } }
                    }
                }
            }
        }
    }
}

/// Builds a self-contained prompt for people who explain questions with their own Claude.
pub fn claude_prompt(q: &Question) -> String {
    let mut p = String::from(
        "Explain this certification practice question. The answer key comes from an ExamTopics dump and is \
         sometimes wrong, so work out the correct answer yourself from official documentation, say whether \
         you agree with the key, and explain why each option is right or wrong.\n\n",
    );
    if let Some(cs) = &q.case_study {
        p.push_str(&format!("Case study:\n{cs}\n\n"));
    }
    p.push_str(&format!("Question ({}):\n{}\n", q.kind.label(), q.body));
    match &q.kind {
        QuestionKind::Choice { options, .. } => {
            p.push('\n');
            for o in options {
                p.push_str(&format!("{}. {}\n", o.key, o.text));
            }
        }
        QuestionKind::Dropdown { groups } => {
            p.push_str("\nAnswer area:\n");
            for g in groups {
                p.push_str(&format!("- {} [{}]\n", g.label, g.options.join(" | ")));
            }
        }
        QuestionKind::DragDrop { items, slots, .. } => {
            p.push_str(&format!("\nItems:\n{}\n", items.join("\n")));
            let labels: Vec<&str> = slots.iter().map(|s| s.label.as_str()).collect();
            p.push_str(&format!("Targets:\n{}\n", labels.join("\n")));
        }
        QuestionKind::Visual { .. } => {
            p.push_str(
                "\n(This question includes an image; I can describe or upload it if needed.)\n",
            );
        }
    }
    p.push_str(&format!(
        "\nAnswer key in the dump: {}\n",
        q.kind.answer_summary()
    ));
    if !q.community_votes.is_empty() {
        let votes: Vec<String> = q
            .community_votes
            .iter()
            .map(|v| format!("{} {}%", v.answer, v.percent))
            .collect();
        p.push_str(&format!("Community vote: {}\n", votes.join(", ")));
    }
    p
}

/// "Use your own Claude": copies a ready-made prompt and opens claude.ai.
#[component]
fn OwnClaude(row: QuestionRow) -> Element {
    let mut copied = use_signal(|| false);
    let prompt = claude_prompt(&row.data);
    rsx! {
        div { class: "stack-sm",
            p { class: "small",
                "Explanations for this app run on the owner's Claude subscription, so they're only generated for invited accounts. "
                "You can get the same explanation with your own Claude account:"
            }
            div { class: "row",
                Button {
                    size: ButtonSize::Sm,
                    onclick: move |_| {
                        let prompt = prompt.clone();
                        async move {
                            if crate::ui::copy_to_clipboard(&prompt).await {
                                copied.set(true);
                            }
                        }
                    },
                    if copied() { Check { size: "14px" } "Prompt copied" } else { "1. Copy prompt" }
                }
                a { href: "https://claude.ai/new", target: "_blank", rel: "noopener noreferrer",
                    Button { size: ButtonSize::Sm, variant: ButtonVariant::Outline, Sparkles { size: "14px" } "2. Open Claude and paste" }
                }
            }
        }
    }
}
