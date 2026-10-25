//! Fix OCR mistakes or a wrong answer key. The original parse is kept in `source`.

use crate::Route;
use crate::components::button::{Button, ButtonVariant};
use crate::components::card::*;
use crate::components::input::Input;
use crate::components::label::Label;
use crate::components::textarea::Textarea;
use crate::data::{self, QuestionRow};
use crate::supabase::auth::SESSION;
use crate::ui::{Banner, ErrorState, Loading};
use dioxus::prelude::*;
use dioxus_primitives::toast::{ToastOptions, use_toast};
use examcore::{ChoiceOption, DragSlot, DropdownGroup, QuestionKind};

#[component]
pub fn EditQuestion(id: String) -> Element {
    let mut loaded = use_resource(use_reactive!(|id| async move {
        let row = data::question(&id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Question not found")?;
        let can_edit = data::can_edit(&row.certification_id).await;
        Ok::<_, String>((row, can_edit))
    }));
    match &*loaded.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => {
            rsx! { ErrorState { message: e.clone(), on_retry: move |_| loaded.restart() } }
        }
        Some(Ok((_, false))) => {
            rsx! { ErrorState { message: "Only the uploader of this certification (or an admin) can edit its questions.".to_string() } }
        }
        Some(Ok((row, true))) => rsx! { Editor { row: row.clone() } },
    }
}

#[component]
fn Editor(row: QuestionRow) -> Element {
    let nav = use_navigator();
    let toast = use_toast();
    let mut body = use_signal(|| row.data.body.clone());
    let mut case_study = use_signal(|| row.data.case_study.clone().unwrap_or_default());
    let mut kind = use_signal(|| row.data.kind.clone());
    let mut kind_json =
        use_signal(|| serde_json::to_string_pretty(&row.data.kind).unwrap_or_default());
    let mut json_error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let back = Route::Quiz {
        id: row.set_id.clone(),
        q: (row.position + 1).to_string(),
    };

    let mut set_kind = move |k: QuestionKind| {
        kind_json.set(serde_json::to_string_pretty(&k).unwrap_or_default());
        json_error.set(None);
        kind.set(k);
    };

    let save = {
        let row = row.clone();
        let back = back.clone();
        move |_| {
            let row = row.clone();
            let back = back.clone();
            async move {
                let Some(user) = SESSION.peek().as_ref().map(|s| s.user.id.clone()) else {
                    return;
                };
                let mut q = row.data.clone();
                q.body = body();
                q.case_study = Some(case_study()).filter(|c| !c.trim().is_empty());
                q.kind = kind();
                busy.set(true);
                let res = data::save_question(&row.id, &q, &user).await;
                busy.set(false);
                match res {
                    Ok(()) => {
                        toast.success("Question saved".into(), ToastOptions::new());
                        nav.push(back);
                    }
                    Err(e) => toast.error(
                        "Couldn't save".into(),
                        ToastOptions::new().description(e.to_string()),
                    ),
                }
            }
        }
    };

    let original = row.source.clone();
    let choice_editor = match kind() {
        QuestionKind::Choice { options, correct } => rsx! {
            div { class: "stack-sm",
                Label { html_for: "", "Options (tick the correct ones)" }
                for (i, opt) in options.iter().enumerate() {
                    {
                        let is_correct = correct.contains(&opt.key);
                        let key = opt.key.clone();
                        rsx! {
                            div { key: "{i}", class: "row", style: "flex-wrap:nowrap",
                                input {
                                    r#type: "checkbox",
                                    checked: is_correct,
                                    onchange: {
                                        let key = key.clone();
                                        move |_| {
                                            if let QuestionKind::Choice { options, mut correct } = kind() {
                                                if correct.contains(&key) { correct.retain(|k| k != &key) } else { correct.push(key.clone()); correct.sort(); }
                                                set_kind(QuestionKind::Choice { options, correct });
                                            }
                                        }
                                    },
                                }
                                span { class: "bold", style: "width:1.5rem", "{opt.key}" }
                                Input {
                                    style: "flex:1",
                                    value: opt.text.clone(),
                                    oninput: move |e: FormEvent| {
                                        if let QuestionKind::Choice { mut options, correct } = kind() {
                                            options[i].text = e.value();
                                            set_kind(QuestionKind::Choice { options, correct });
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                div {
                    Button {
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| {
                            if let QuestionKind::Choice { mut options, correct } = kind() {
                                let key = ((b'A' + options.len() as u8) as char).to_string();
                                options.push(ChoiceOption { key, text: String::new() });
                                set_kind(QuestionKind::Choice { options, correct });
                            }
                        },
                        "+ Add option"
                    }
                }
            }
        },
        _ => rsx! {},
    };

    rsx! {
        div { class: "container narrow stack",
            div { class: "page-head",
                div {
                    h1 { "Edit question #{row.number}" }
                    p { class: "muted", "Fix OCR mistakes or a wrong answer key. Changes apply for everyone; the original PDF text is kept." }
                }
            }
            Card {
                CardContent { style: "padding-top:1.25rem",
                    div { class: "stack",
                        div { class: "field",
                            Label { html_for: "body", "Question text" }
                            Textarea { id: "body", rows: "8", value: body(), oninput: move |e: FormEvent| body.set(e.value()) }
                        }
                        div { class: "field",
                            Label { html_for: "cs", "Case study (optional)" }
                            Textarea { id: "cs", rows: "3", value: case_study(), oninput: move |e: FormEvent| case_study.set(e.value()) }
                        }
                        {choice_editor}
                        details { class: "case-study",
                            summary { "Advanced: question type & answer (JSON)" }
                            div { class: "content stack-sm", style: "white-space:normal",
                                p { class: "small muted", "Turn an image question into an interactive one by switching its type:" }
                                div { class: "row",
                                    Button { variant: ButtonVariant::Outline, onclick: move |_| set_kind(dropdown_template()), "Hotspot dropdowns" }
                                    Button { variant: ButtonVariant::Outline, onclick: move |_| set_kind(dragdrop_template(false)), "Drag & drop" }
                                    Button { variant: ButtonVariant::Outline, onclick: move |_| set_kind(dragdrop_template(true)), "Ordering" }
                                }
                                Textarea {
                                    class: "code",
                                    value: kind_json(),
                                    oninput: move |e: FormEvent| {
                                        kind_json.set(e.value());
                                        match serde_json::from_str::<QuestionKind>(&e.value()) {
                                            Ok(k) => { kind.set(k); json_error.set(None); }
                                            Err(err) => json_error.set(Some(err.to_string())),
                                        }
                                    },
                                }
                                if let Some(e) = json_error() { p { class: "error-text", "{e}" } }
                            }
                        }
                        if row.key_edited() || row.edited_at.is_some() {
                            Banner { kind: "info",
                                div { class: "row-between full",
                                    span { class: "small", "Original answer from the PDF: {original.kind.answer_summary()}" }
                                    button {
                                        class: "link small",
                                        onclick: {
                                            let original = original.clone();
                                            move |_| {
                                                body.set(original.body.clone());
                                                case_study.set(original.case_study.clone().unwrap_or_default());
                                                set_kind(original.kind.clone());
                                            }
                                        },
                                        "Revert to original"
                                    }
                                }
                            }
                        }
                    }
                }
                CardFooter {
                    div { class: "row-between full",
                        Link { to: back.clone(), Button { variant: ButtonVariant::Ghost, "Cancel" } }
                        Button { disabled: busy() || json_error().is_some(), onclick: save, "Save" }
                    }
                }
            }
        }
    }
}

fn dropdown_template() -> QuestionKind {
    QuestionKind::Dropdown {
        groups: vec![
            DropdownGroup {
                label: "Box 1 / statement 1".into(),
                options: vec!["Yes".into(), "No".into()],
                correct: Some(0),
            },
            DropdownGroup {
                label: "Box 2 / statement 2".into(),
                options: vec!["Yes".into(), "No".into()],
                correct: Some(1),
            },
        ],
    }
}

fn dragdrop_template(ordered: bool) -> QuestionKind {
    QuestionKind::DragDrop {
        items: vec!["Action A".into(), "Action B".into(), "Action C".into()],
        slots: (0..3)
            .map(|i| DragSlot {
                label: if ordered {
                    format!("Step {}", i + 1)
                } else {
                    format!("Target {}", i + 1)
                },
                correct: Some(i),
            })
            .collect(),
        ordered,
    }
}
