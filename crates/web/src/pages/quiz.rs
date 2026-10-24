//! Practice a set: one question at a time, resume where you left off,
//! check answers and read Claude's explanation.

use super::question::{AnswerKeyInfo, ExplanationPanel, QuestionView, SourceExplanation};
use crate::Route;
use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::card::*;
use crate::data::{self, Certification, ExamSet, ExplanationRow, JobRow, ProgressRow, QuestionRow};
use crate::supabase::auth::SESSION;
use crate::ui::{Banner, ErrorState, Loading, Meter, percent};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ArrowLeft, ArrowRight, ChevronLeft, PencilLine, RotateCcw, SkipForward,
};
use dioxus_primitives::toast::{ToastOptions, use_toast};
use examcore::{QuestionKind, Response, grade};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, PartialEq)]
struct Loaded {
    set: ExamSet,
    cert: Option<Certification>,
    questions: Vec<QuestionRow>,
    progress: Vec<ProgressRow>,
    resume: i32,
    images: HashMap<String, String>,
    explanations: HashMap<String, ExplanationRow>,
    jobs: HashMap<String, JobRow>,
    can_edit: bool,
    /// On the explainer allowlist (the owner's Claude subscription).
    can_explain: bool,
}

async fn load(id: String) -> Result<Loaded, String> {
    let e = |e: crate::supabase::ApiError| e.to_string();
    let set = data::exam_set(&id)
        .await
        .map_err(e)?
        .ok_or("This set doesn't exist (anymore).")?;
    let (questions, progress, stats, cert) = futures_util::join!(
        data::questions(&id),
        data::progress(&id),
        data::my_stats(Some(&set.certification_id)),
        data::certification(&set.certification_id),
    );
    let questions = questions.map_err(e)?;
    let ids: Vec<String> = questions.iter().map(|q| q.id.clone()).collect();
    let (explanations, jobs, images, can_edit, can_explain) = futures_util::join!(
        data::explanations(&ids),
        data::jobs(&ids),
        data::image_urls(&questions),
        data::can_edit(&set.certification_id),
        data::can_use_explainer(),
    );
    let resume = stats
        .unwrap_or_default()
        .into_iter()
        .find(|s| s.set_id == id)
        .and_then(|s| s.current_position)
        .unwrap_or(0);
    Ok(Loaded {
        set,
        cert: cert.ok().flatten(),
        questions,
        progress: progress.map_err(e)?,
        resume,
        images,
        explanations: explanations.unwrap_or_default(),
        jobs: jobs.unwrap_or_default(),
        can_edit,
        can_explain,
    })
}

#[component]
pub fn Quiz(id: String, q: String) -> Element {
    let mut loaded = use_resource(use_reactive!(|id| load(id)));
    match &*loaded.read() {
        None => rsx! { Loading { label: "Loading questions…" } },
        Some(Err(e)) => {
            rsx! { ErrorState { message: e.clone(), on_retry: move |_| loaded.restart() } }
        }
        Some(Ok(l)) if l.questions.is_empty() => rsx! {
            ErrorState { message: "This set has no questions.".to_string() }
        },
        Some(Ok(l)) => {
            // ?q=<number> (1-based) wins over the saved resume position.
            let start = q
                .parse::<i32>()
                .ok()
                .map(|n| n - 1)
                .unwrap_or(l.resume)
                .clamp(0, l.questions.len() as i32 - 1) as usize;
            rsx! { QuizInner { key: "{l.set.id}", data: l.clone(), start } }
        }
    }
}

#[component]
fn QuizInner(data: Loaded, start: usize) -> Element {
    let toast = use_toast();
    let questions = data.questions.clone();
    let total = questions.len();
    let set_id = data.set.id.clone();
    let can_explain = data.can_explain;
    let mut pos = use_signal(|| start);
    let mut drafts = use_signal(HashMap::<String, Response>::new);
    let mut results = use_signal(|| {
        data.progress
            .iter()
            .map(|p| (p.question_id.clone(), p.clone()))
            .collect::<HashMap<_, _>>()
    });
    // Questions currently shown with their result (answered before or just checked).
    let mut revealed = use_signal(|| {
        results
            .peek()
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
    });
    let mut explanations = use_signal(|| data.explanations.clone());
    let mut jobs = use_signal(|| data.jobs.clone());
    let mut overrides = use_signal(HashMap::<String, QuestionRow>::new);
    let mut saving = use_signal(|| false);

    let current = {
        let base = &questions[pos().min(total - 1)];
        overrides
            .read()
            .get(&base.id)
            .cloned()
            .unwrap_or_else(|| base.clone())
    };
    let qid = current.id.clone();
    let is_revealed = revealed.read().contains(&qid);
    let response = drafts
        .read()
        .get(&qid)
        .cloned()
        .or_else(|| results.read().get(&qid).map(|r| r.response.clone()))
        .unwrap_or_else(|| Response::empty_for(&current.data.kind));
    let result = results.read().get(&qid).cloned();

    let answered = results.read().len() as i32;
    let correct = results.read().values().filter(|r| r.is_correct).count() as i32;

    // Persist the position whenever the user navigates.
    {
        let set_id = set_id.clone();
        use_effect(move || {
            let p = pos() as i32;
            let set_id = set_id.clone();
            spawn(async move {
                let _ = data::save_position(&set_id, p).await;
            });
            if let Some(w) = web_sys::window() {
                w.scroll_to_with_x_and_y(0.0, 0.0);
            }
        });
    }

    // Poll for the explanation of the current question while it is being generated.
    {
        let questions = questions.clone();
        use_future(move || {
            let questions = questions.clone();
            async move {
                loop {
                    gloo_timers::future::sleep(Duration::from_secs(6)).await;
                    let id = questions[pos.peek().min(questions.len() - 1)].id.clone();
                    let waiting = revealed.peek().contains(&id)
                        && !explanations.peek().contains_key(&id)
                        && jobs
                            .peek()
                            .get(&id)
                            .is_some_and(|j| j.status == "pending" || j.status == "processing");
                    if !waiting {
                        continue;
                    }
                    let ids = vec![id.clone()];
                    if let Ok(found) = data::explanations(&ids).await {
                        explanations.write().extend(found);
                    }
                    if let Ok(found) = data::jobs(&ids).await {
                        let mut j = jobs.write();
                        j.remove(&id);
                        j.extend(found);
                    }
                }
            }
        });
    }

    let submit = {
        let current = current.clone();
        move |resp: Response| {
            let current = current.clone();
            async move {
                let ok = grade(&current.data.kind, &resp);
                saving.set(true);
                let next_pos = (pos() + 1).min(total - 1) as i32;
                let res = data::record_answer(&current.id, &resp, ok, next_pos).await;
                saving.set(false);
                if let Err(e) = res {
                    toast.error(
                        "Couldn't save your answer".into(),
                        ToastOptions::new().description(e.to_string()),
                    );
                    return;
                }
                let attempts = results
                    .peek()
                    .get(&current.id)
                    .map(|r| r.attempts + 1)
                    .unwrap_or(1);
                results.write().insert(
                    current.id.clone(),
                    ProgressRow {
                        question_id: current.id.clone(),
                        response: resp,
                        is_correct: ok,
                        attempts,
                    },
                );
                drafts.write().remove(&current.id);
                revealed.write().insert(current.id.clone());
                // Answers only queue an explanation for allowlisted users (see record_answer).
                if can_explain
                    && !explanations.peek().contains_key(&current.id)
                    && !jobs.peek().contains_key(&current.id)
                {
                    jobs.write().insert(
                        current.id.clone(),
                        JobRow {
                            question_id: current.id.clone(),
                            status: "pending".into(),
                            error: None,
                        },
                    );
                }
            }
        }
    };
    let submit_check = submit.clone();
    let submit_self = submit.clone();

    let request_explanation = {
        let qid = qid.clone();
        move |force: bool| {
            let qid = qid.clone();
            async move {
                match data::request_explanations(std::slice::from_ref(&qid), force).await {
                    Ok(_) => {
                        if force {
                            explanations.write().remove(&qid);
                        }
                        jobs.write().insert(
                            qid.clone(),
                            JobRow {
                                question_id: qid.clone(),
                                status: "pending".into(),
                                error: None,
                            },
                        );
                    }
                    Err(e) => toast.error(
                        "Couldn't queue the explanation".into(),
                        ToastOptions::new().description(e.to_string()),
                    ),
                }
            }
        }
    };

    let apply_answer = {
        let current = current.clone();
        move |letters: String| {
            let current = current.clone();
            async move {
                let Some(user) = SESSION.peek().as_ref().map(|s| s.user.id.clone()) else {
                    return;
                };
                let Some(updated) = data::with_choice_answer(&current.data, &letters) else {
                    return;
                };
                match data::save_question(&current.id, &updated, &user).await {
                    Ok(()) => {
                        let mut row = current.clone();
                        row.data = updated;
                        overrides.write().insert(row.id.clone(), row);
                        toast.success(
                            "Answer key updated".into(),
                            ToastOptions::new().description("Grading now uses Claude's answer."),
                        );
                    }
                    Err(e) => toast.error(
                        "Couldn't update the answer".into(),
                        ToastOptions::new().description(e.to_string()),
                    ),
                }
            }
        }
    };

    let retry = {
        let qid = qid.clone();
        move |_| {
            revealed.write().remove(&qid);
            drafts.write().remove(&qid);
        }
    };

    let next_unanswered = {
        let questions = questions.clone();
        move || {
            let r = results.read();
            (pos() + 1..total)
                .chain(0..pos())
                .find(|&i| !r.contains_key(&questions[i].id))
        }
    };
    let next_unanswered_idx = next_unanswered();
    let is_visual = matches!(current.data.kind, QuestionKind::Visual { .. });
    let complete = response.is_complete(&current.data.kind);
    let score_kind: &'static str = if percent(correct, answered) >= 70 {
        "ok"
    } else {
        "warn"
    };
    let cert_code = data
        .cert
        .as_ref()
        .map(|c| c.code.clone())
        .unwrap_or_default();

    rsx! {
        div { class: "container",
            div { class: "row", style: "margin-bottom:1rem",
                Link { class: "link small", to: Route::CertificationPage { id: data.set.certification_id.clone() },
                    span { class: "row", style: "gap:.25rem", ChevronLeft { size: "15px" } "{cert_code}" }
                }
                span { class: "muted small", "/" }
                span { class: "small bold", "{data.set.title}" }
            }
            div { class: "quiz",
                aside { class: "quiz-side",
                    Card {
                        CardContent { style: "padding-top:1rem",
                            div { class: "stack-sm",
                                div { class: "row-between small",
                                    span { class: "bold", "{answered}/{total} answered" }
                                    span { class: "muted", "{percent(correct, answered)}% correct" }
                                }
                                Meter { total: total as i32, answered, correct }
                                div { class: "qnav",
                                    for (i, qrow) in questions.iter().enumerate() {
                                        {
                                            let status = match results.read().get(&qrow.id) {
                                                Some(r) if r.is_correct => "correct",
                                                Some(_) => "incorrect",
                                                None => "",
                                            };
                                            rsx! {
                                                button {
                                                    key: "{qrow.id}",
                                                    title: "Question #{qrow.number}",
                                                    "data-status": status,
                                                    "data-current": "{i == pos()}",
                                                    onclick: move |_| pos.set(i),
                                                    "{i + 1}"
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "legend",
                                    span { i { class: "lc" } "Correct" }
                                    span { i { class: "li" } "Incorrect" }
                                    span { i {} "Unanswered" }
                                }
                            }
                        }
                    }
                    if answered as usize == total {
                        Banner { kind: score_kind,
                            div { class: "stack-sm",
                                span { class: "bold", "Set complete: {correct}/{total} ({percent(correct, total as i32)}%)" }
                                span { class: "small", "Review the red ones, or retry them from the question page." }
                            }
                        }
                    }
                }
                section { class: "stack",
                    Card {
                        CardHeader {
                            div { class: "q-head",
                                h2 { "Question {pos() + 1}" }
                                span { class: "muted small", "#{current.number} · Topic {current.topic}" }
                                Badge { variant: BadgeVariant::Secondary, "{current.data.kind.label()}" }
                                if current.key_edited() { Badge { variant: BadgeVariant::Outline, "Answer corrected" } }
                            }
                            if data.can_edit {
                                CardAction {
                                    Link { to: Route::EditQuestion { id: current.id.clone() },
                                        Button { size: ButtonSize::Sm, variant: ButtonVariant::Ghost, PencilLine { size: "14px" } "Edit" }
                                    }
                                }
                            }
                        }
                        CardContent {
                            div { class: "stack",
                                if let Some(intro) = &data.set.intro {
                                    details { class: "case-study",
                                        summary { "Case study" }
                                        div { class: "content", "{intro}" }
                                    }
                                }
                                QuestionView {
                                    key: "{current.id}",
                                    row: current.clone(),
                                    images: data.images.clone(),
                                    response: response.clone(),
                                    checked: is_revealed,
                                    on_change: {
                                        let qid = qid.clone();
                                        move |r: Response| { drafts.write().insert(qid.clone(), r); }
                                    },
                                    on_self_grade: move |ok: bool| submit_self(Response::SelfGraded(ok)),
                                }
                                if is_revealed {
                                    if let Some(r) = &result {
                                        Banner { kind: result_kind(r.is_correct),
                                            div { class: "stack-sm",
                                                span { class: "bold",
                                                    if r.is_correct { "Correct!" } else { "Not quite." }
                                                    if r.attempts > 1 { span { class: "muted small", " (attempt {r.attempts})" } }
                                                }
                                                if !r.is_correct {
                                                    span { class: "small", "Correct answer: {current.data.kind.answer_summary()}" }
                                                }
                                            }
                                        }
                                    }
                                    AnswerKeyInfo { row: current.clone() }
                                }
                                div { class: "quiz-actions",
                                    Button {
                                        variant: ButtonVariant::Outline,
                                        disabled: pos() == 0,
                                        onclick: move |_| pos.set(pos().saturating_sub(1)),
                                        ArrowLeft { size: "16px" } "Previous"
                                    }
                                    div { class: "row",
                                        if is_revealed {
                                            Button { variant: ButtonVariant::Ghost, onclick: retry, RotateCcw { size: "16px" } "Try again" }
                                            if let Some(n) = next_unanswered_idx {
                                                if n != pos() + 1 {
                                                    Button { variant: ButtonVariant::Ghost, onclick: move |_| pos.set(n), SkipForward { size: "16px" } "Next unanswered" }
                                                }
                                            }
                                            Button {
                                                disabled: pos() + 1 >= total,
                                                onclick: move |_| pos.set((pos() + 1).min(total - 1)),
                                                "Next" ArrowRight { size: "16px" }
                                            }
                                        } else if !is_visual {
                                            Button {
                                                variant: ButtonVariant::Ghost,
                                                disabled: pos() + 1 >= total,
                                                onclick: move |_| pos.set((pos() + 1).min(total - 1)),
                                                "Skip"
                                            }
                                            Button {
                                                disabled: !complete || saving(),
                                                onclick: {
                                                    let response = response.clone();
                                                    let submit_check = submit_check.clone();
                                                    move |_| submit_check(response.clone())
                                                },
                                                "Check answer"
                                            }
                                        } else {
                                            Button {
                                                variant: ButtonVariant::Ghost,
                                                disabled: pos() + 1 >= total,
                                                onclick: move |_| pos.set((pos() + 1).min(total - 1)),
                                                "Skip"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if is_revealed {
                        ExplanationPanel {
                            key: "{current.id}-exp",
                            row: current.clone(),
                            explanation: explanations.read().get(&qid).cloned(),
                            job: jobs.read().get(&qid).cloned(),
                            can_edit: data.can_edit,
                            can_request: data.can_explain,
                            on_request: request_explanation,
                            on_apply_answer: apply_answer,
                        }
                        SourceExplanation { q: current.data.clone() }
                    }
                    if !current.parse_warnings.is_empty() && data.can_edit {
                        Banner { kind: "warn",
                            div { class: "stack-sm small",
                                span { class: "bold", "Import warnings" }
                                for w in current.parse_warnings.iter() { span { "{w}" } }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn result_kind(ok: bool) -> &'static str {
    if ok { "ok" } else { "bad" }
}
