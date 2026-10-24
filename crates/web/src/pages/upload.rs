//! Upload an ExamTopics PDF: read its text layer with pdf.js, parse it into questions,
//! review the result, then import it as sets of a certification.

use crate::Route;
use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonVariant};
use crate::components::card::*;
use crate::components::input::Input;
use crate::components::label::Label;
use crate::components::switch::Switch;
use crate::data::{self, Certification};
use crate::pdf::{self, PdfInfo};
use crate::supabase::auth::SESSION;
use crate::supabase::{ApiError, db, storage};
use crate::ui::{Banner, Spinner};
use dioxus::prelude::*;
use dioxus_icons::lucide::{FileText, Upload as UploadIcon};
use examcore::parse::{ParseOutput, ParsedQuestion, plan_sets};
use examcore::{QuestionKind, VisualVariant};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, PartialEq)]
enum Stage {
    Pick,
    Reading {
        done: u32,
        total: u32,
    },
    Review,
    Importing {
        label: String,
        done: usize,
        total: usize,
    },
    Done {
        certification_id: String,
        imported: usize,
    },
}

const INPUT_ID: &str = "pdf-input";

#[component]
pub fn Upload() -> Element {
    let is_guest = SESSION.read().as_ref().is_some_and(|s| s.user.is_anonymous);
    let mut stage = use_signal(|| Stage::Pick);
    let mut info = use_signal(|| None::<PdfInfo>);
    let mut parsed = use_signal(|| None::<ParseOutput>);
    let mut error = use_signal(|| None::<String>);

    // Review form
    let mut target = use_signal(|| "new".to_string());
    let mut code = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut vendor = use_signal(String::new);
    let mut chunk = use_signal(|| "50".to_string());
    let mut snapshots_all = use_signal(|| true);
    let mut queue_explanations = use_signal(|| false);
    let mut only_warnings = use_signal(|| false);

    let editable = use_resource(|| async {
        let me = SESSION
            .peek()
            .as_ref()
            .map(|s| s.user.id.clone())
            .unwrap_or_default();
        let admin = data::profile(&me)
            .await
            .ok()
            .flatten()
            .is_some_and(|p| p.is_admin);
        data::certifications()
            .await
            .map(|all| {
                all.into_iter()
                    .filter(|c| admin || c.created_by.as_deref() == Some(&me))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    if is_guest {
        return rsx! {
            div { class: "container narrow",
                Banner { kind: "info",
                    div { class: "stack-sm",
                        span { class: "bold", "Sign in to upload" }
                        span { class: "small", "Guests can study, but uploading requires an account so the certification has an owner who can maintain it." }
                        Link { class: "link small", to: Route::Account { reset: String::new() }, "Link an account →" }
                    }
                }
            }
        };
    }

    let on_file = move |_| async move {
        error.set(None);
        parsed.set(None);
        let opened = match pdf::open(INPUT_ID).await {
            Ok(i) => i,
            Err(e) => {
                error.set(Some(format!("Couldn't open the PDF: {e}")));
                return;
            }
        };
        let total = opened.pages;
        info.set(Some(opened.clone()));
        stage.set(Stage::Reading { done: 0, total });
        let mut pages = Vec::with_capacity(total as usize);
        for i in 0..total {
            match pdf::page_text(i).await {
                Ok(p) => pages.push(p),
                Err(e) => {
                    error.set(Some(format!("Couldn't read page {}: {e}", i + 1)));
                    stage.set(Stage::Pick);
                    return;
                }
            }
            if i % 5 == 0 || i + 1 == total {
                stage.set(Stage::Reading { done: i + 1, total });
                // Let the browser paint the progress.
                gloo_timers::future::sleep(std::time::Duration::from_millis(0)).await;
            }
        }
        let out = examcore::parse::parse_pages(&pages);
        if out.questions.is_empty() {
            let has_text = pages.iter().any(|p| !p.items.is_empty());
            error.set(Some(if has_text {
                "No ExamTopics questions were found (looking for \"Question #N\" headers). Is this an ExamTopics export?".into()
            } else {
                "This PDF has no text layer. Run OCR on it first (e.g. Adobe Acrobat → Scan & OCR, or `ocrmypdf in.pdf out.pdf`).".into()
            }));
            stage.set(Stage::Pick);
            return;
        }
        let file_stem = opened.name.trim_end_matches(".pdf").to_string();
        // Exports often have no title page; the file name ("21042026 SC-100 OCR.pdf")
        // usually still carries the exam code.
        let exam_code = out
            .exam_code
            .clone()
            .or_else(|| examcore::parse::guess_exam_code(&file_stem));
        name.set(
            out.title
                .clone()
                .map(|t| t.chars().take(120).collect())
                .or_else(|| exam_code.clone())
                .unwrap_or_else(|| file_stem.clone()),
        );
        code.set(exam_code.unwrap_or(file_stem));
        parsed.set(Some(out));
        stage.set(Stage::Review);
    };

    let import = move |_| async move {
        let Some(out) = parsed() else { return };
        let Some(pdf_info) = info() else { return };
        error.set(None);
        let opts = ImportOptions {
            existing: (target() != "new").then(|| target()),
            code: code().trim().to_string(),
            name: name().trim().to_string(),
            vendor: Some(vendor().trim().to_string()).filter(|v| !v.is_empty()),
            chunk: chunk().trim().parse().unwrap_or(50),
            snapshots_all: snapshots_all(),
            queue_explanations: queue_explanations(),
            filename: pdf_info.name.clone(),
        };
        if opts.existing.is_none() && (opts.code.is_empty() || opts.name.is_empty()) {
            error.set(Some("Please enter an exam code and a name.".into()));
            return;
        }
        match run_import(out, opts, stage).await {
            Ok((certification_id, imported)) => {
                pdf::close().await;
                stage.set(Stage::Done {
                    certification_id,
                    imported,
                });
            }
            Err(e) => {
                error.set(Some(e.to_string()));
                stage.set(Stage::Review);
            }
        }
    };

    let body = match stage() {
        Stage::Pick => rsx! {
            label { class: "file-drop", r#for: INPUT_ID,
                UploadIcon { size: "28px" }
                span { class: "bold", "Choose an ExamTopics PDF" }
                span { class: "small muted", "The PDF must have a text layer (OCR). Everything is processed in your browser." }
                input { id: INPUT_ID, r#type: "file", accept: "application/pdf,.pdf", onchange: on_file }
            }
        },
        Stage::Reading { done, total } => rsx! {
            Card {
                CardContent { style: "padding-top:1.25rem",
                    div { class: "stack-sm",
                        div { class: "row", Spinner {} span { "Reading page {done} of {total}…" } }
                        progress { class: "full", max: "{total}", value: "{done}" }
                    }
                }
            }
        },
        Stage::Importing { label, done, total } => rsx! {
            Card {
                CardContent { style: "padding-top:1.25rem",
                    div { class: "stack-sm",
                        div { class: "row", Spinner {} span { "{label}" } }
                        progress { class: "full", max: "{total.max(1)}", value: "{done}" }
                        span { class: "tiny muted", "Keep this tab open until the import finishes." }
                    }
                }
            }
        },
        Stage::Done {
            certification_id,
            imported,
        } => rsx! {
            Banner { kind: "ok",
                div { class: "stack-sm",
                    span { class: "bold", "Imported {imported} questions" }
                    Link { class: "link", to: Route::CertificationPage { id: certification_id }, "Open the certification →" }
                }
            }
        },
        Stage::Review => {
            let Some(out) = parsed() else { return rsx! {} };
            let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
            for q in &out.questions {
                *by_kind.entry(q.question.kind.label()).or_default() += 1;
            }
            let warned = out
                .questions
                .iter()
                .filter(|q| !q.warnings.is_empty())
                .count();
            let plans = plan_sets(&out, chunk().trim().parse().unwrap_or(50));
            let certs = editable.read().clone().unwrap_or_default();
            rsx! {
                Card {
                    CardHeader {
                        CardTitle {
                            span { class: "row", style: "gap:.5rem", FileText { size: "18px" } "{info().map(|i| i.name).unwrap_or_default()}" }
                        }
                        CardDescription {
                            "{out.questions.len()} questions · {out.topics.len()} topics · {info().map(|i| i.pages).unwrap_or(0)} pages"
                        }
                    }
                    CardContent {
                        div { class: "stack",
                            div { class: "row", style: "gap:.4rem",
                                for (k, n) in by_kind.iter() { Badge { variant: BadgeVariant::Secondary, "{n} × {k}" } }
                                if warned > 0 { Badge { variant: BadgeVariant::Destructive, "{warned} with warnings" } }
                            }
                            if by_kind.contains_key("Hotspot") || by_kind.contains_key("Drag & drop") || by_kind.contains_key("Image") {
                                p { class: "small muted",
                                    "Image-based questions (hotspot / drag & drop) are shown as page snapshots. "
                                    "The Claude explainer converts them into interactive dropdowns and drag & drop when it processes them."
                                }
                            }
                        }
                    }
                }
                Card {
                    CardHeader { CardTitle { "Where should it go?" } }
                    CardContent {
                        div { class: "stack",
                            div { class: "field",
                                Label { html_for: "target", "Certification" }
                                select {
                                    id: "target",
                                    class: "native-select",
                                    value: target(),
                                    onchange: move |e| target.set(e.value()),
                                    option { value: "new", "Create a new certification" }
                                    for c in certs.iter() {
                                        option { value: "{c.id}", "Add to {c.code} — {c.name}" }
                                    }
                                }
                            }
                            if target() == "new" {
                                div { class: "field-row",
                                    div { class: "field",
                                        Label { html_for: "code", "Exam code" }
                                        Input { id: "code", value: code(), placeholder: "AZ-104", oninput: move |e: FormEvent| code.set(e.value()) }
                                    }
                                    div { class: "field",
                                        Label { html_for: "vendor", "Vendor (optional)" }
                                        Input { id: "vendor", value: vendor(), placeholder: "Microsoft", oninput: move |e: FormEvent| vendor.set(e.value()) }
                                    }
                                }
                                div { class: "field",
                                    Label { html_for: "name", "Name" }
                                    Input { id: "name", value: name(), placeholder: "Microsoft Azure Administrator", oninput: move |e: FormEvent| name.set(e.value()) }
                                }
                            }
                            div { class: "field",
                                Label { html_for: "chunk", "Questions per set" }
                                Input {
                                    id: "chunk", r#type: "number", min: "0", step: "5", value: chunk(),
                                    oninput: move |e: FormEvent| chunk.set(e.value()),
                                }
                                span { class: "tiny muted", "Sets follow the PDF's topics; large topics are split into sets of this size (0 = one set per topic)." }
                            }
                            div { class: "stack-sm small",
                                for plan in plans.iter() {
                                    div { class: "row-between",
                                        span { "{plan.title}" }
                                        span { class: "muted", "{plan.end - plan.start} questions" }
                                    }
                                }
                            }
                            hr { class: "divider" }
                            label { class: "row-between",
                                div { class: "stack-sm", style: "gap:0",
                                    span { class: "bold small", "Save page snapshots for every question" }
                                    span { class: "tiny muted", "Lets everyone compare with the original PDF. Image questions always get snapshots." }
                                }
                                Switch { checked: snapshots_all(), on_checked_change: move |v| snapshots_all.set(v) }
                            }
                            label { class: "row-between",
                                div { class: "stack-sm", style: "gap:0",
                                    span { class: "bold small", "Queue Claude explanations for all questions now" }
                                    span { class: "tiny muted", "Otherwise explanations are generated as people answer questions." }
                                }
                                Switch { checked: queue_explanations(), on_checked_change: move |v| queue_explanations.set(v) }
                            }
                        }
                    }
                    CardFooter {
                        div { class: "row-between full",
                            Button { variant: ButtonVariant::Ghost, onclick: move |_| { parsed.set(None); stage.set(Stage::Pick); }, "Choose another file" }
                            Button { onclick: import, "Import {out.questions.len()} questions" }
                        }
                    }
                }
                Card {
                    CardHeader {
                        CardTitle { "Preview" }
                        CardAction {
                            label { class: "row small", style: "gap:.5rem",
                                "Only with warnings"
                                Switch { checked: only_warnings(), on_checked_change: move |v| only_warnings.set(v) }
                            }
                        }
                    }
                    CardContent {
                        div { class: "preview-list",
                            for (i, q) in out.questions.iter().enumerate().filter(|(_, q)| !only_warnings() || !q.warnings.is_empty()).take(400) {
                                PreviewItem { key: "{i}", q: q.clone() }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "container narrow stack",
            div { class: "page-head",
                div {
                    h1 { "Upload a PDF" }
                    p { class: "muted", "Turn an OCR'd ExamTopics PDF into interactive practice sets." }
                }
            }
            if let Some(e) = error() {
                Banner { kind: "bad", span { class: "small", "{e}" } }
            }
            {body}
        }
    }
}

#[component]
fn PreviewItem(q: ParsedQuestion) -> Element {
    let question = &q.question;
    let preview: String = question.body.chars().take(140).collect();
    rsx! {
        details { class: "preview-item",
            summary {
                span { class: "bold", "#{question.number}" }
                span { class: "muted tiny", "T{question.topic}" }
                Badge { variant: BadgeVariant::Outline, "{question.kind.label()}" }
                span { class: "small", style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:1", "{preview}" }
                if !q.warnings.is_empty() { Badge { variant: BadgeVariant::Destructive, "!" } }
            }
            div { class: "body", "{question.body}" }
            if let QuestionKind::Choice { options, correct } = &question.kind {
                div { class: "body",
                    for o in options.iter() { div { "{o.key}. {o.text}" } }
                    div { class: "bold", "Answer: {correct.join(\"\")}" }
                }
            } else {
                div { class: "body bold", "Answer: {question.kind.answer_summary()}" }
            }
            if !q.warnings.is_empty() {
                ul { class: "warn-list", for w in q.warnings.iter() { li { "{w}" } } }
            }
        }
    }
}

struct ImportOptions {
    existing: Option<String>,
    code: String,
    name: String,
    vendor: Option<String>,
    chunk: usize,
    snapshots_all: bool,
    queue_explanations: bool,
    filename: String,
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    if let Some(c) = web_sys::window().and_then(|w| w.crypto().ok()) {
        let _ = c.get_random_values_with_u8_array(&mut b);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn needs_snapshot(q: &ParsedQuestion) -> bool {
    q.question.has_exhibit
        || matches!(
            q.question.kind,
            QuestionKind::Visual { .. } | QuestionKind::Dropdown { .. }
        )
}

async fn run_import(
    out: ParseOutput,
    opts: ImportOptions,
    mut stage: Signal<Stage>,
) -> Result<(String, usize), ApiError> {
    #[derive(Deserialize)]
    struct IdRow {
        id: String,
    }
    #[derive(Deserialize)]
    struct PosRow {
        position: i32,
    }

    stage.set(Stage::Importing {
        label: "Creating certification…".into(),
        done: 0,
        total: 1,
    });
    let (cert_id, mut set_offset, existing_count) = match &opts.existing {
        Some(id) => {
            let last: Option<PosRow> = db::select_one(
                "exam_sets",
                &format!("select=position&certification_id=eq.{id}&order=position.desc"),
            )
            .await?;
            let cert: Option<Certification> = data::certification(id).await?;
            (
                id.clone(),
                last.map(|l| l.position + 1).unwrap_or(0),
                cert.map(|c| c.question_count).unwrap_or(0),
            )
        }
        None => {
            let me = SESSION
                .peek()
                .as_ref()
                .map(|s| s.user.id.clone())
                .unwrap_or_default();
            let rows: Vec<IdRow> = db::insert_returning(
                "certifications",
                &json!({
                    "code": opts.code,
                    "name": opts.name,
                    "vendor": opts.vendor,
                    "source_filename": opts.filename,
                    "created_by": me,
                }),
                "id",
            )
            .await?;
            (
                rows.into_iter()
                    .next()
                    .ok_or_else(|| ApiError::msg("Certification was not created"))?
                    .id,
                0,
                0,
            )
        }
    };

    let plans = plan_sets(&out, opts.chunk);
    let total = out.questions.len();
    let mut done = 0usize;
    let mut all_ids = Vec::with_capacity(total);

    for plan in &plans {
        let set_rows: Vec<IdRow> = db::insert_returning(
            "exam_sets",
            &json!({
                "certification_id": cert_id,
                "title": plan.title,
                "topic": plan.topic,
                "position": set_offset,
                "question_count": plan.end - plan.start,
            }),
            "id",
        )
        .await?;
        set_offset += 1;
        let set_id = set_rows
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::msg("Set was not created"))?
            .id;

        let questions = &out.questions[plan.start..plan.end];
        for (batch_index, batch) in questions.chunks(10).enumerate() {
            let mut rows = Vec::with_capacity(batch.len());
            let mut uploads = Vec::new();
            for (i, pq) in batch.iter().enumerate() {
                let position = batch_index * 10 + i;
                let id = uuid_v4();
                let mut question = pq.question.clone();
                stage.set(Stage::Importing {
                    label: format!("Importing question {} of {total}…", done + 1),
                    done,
                    total,
                });
                if opts.snapshots_all || needs_snapshot(pq) {
                    if let Ok(Some(snap)) = pdf::snapshot(&pq.question_regions, 1.6).await {
                        let path = format!("{cert_id}/{id}-q.{}", snap.extension());
                        question.images.question = Some(path.clone());
                        uploads.push((path, snap));
                    }
                    let wants_answer = opts.snapshots_all
                        || matches!(
                            question.kind,
                            QuestionKind::Visual { .. } | QuestionKind::Dropdown { .. }
                        );
                    if wants_answer
                        && let Ok(Some(snap)) = pdf::snapshot(&pq.answer_regions, 1.6).await
                    {
                        let path = format!("{cert_id}/{id}-a.{}", snap.extension());
                        question.images.answer = Some(path.clone());
                        uploads.push((path, snap));
                    }
                }
                if matches!(
                    question.kind,
                    QuestionKind::Visual {
                        variant: VisualVariant::Other,
                        ..
                    }
                ) && question.images.question.is_some()
                {
                    question.has_exhibit = true;
                }
                rows.push(json!({
                    "id": id,
                    "certification_id": cert_id,
                    "set_id": set_id,
                    "position": position,
                    "number": question.number,
                    "topic": question.topic,
                    "data": question,
                    "source": question,
                    "parse_warnings": pq.warnings,
                }));
                all_ids.push(id);
                done += 1;
            }
            let results = futures_util::future::join_all(uploads.into_iter().map(
                |(path, snap)| async move {
                    storage::upload(&path, snap.bytes, &snap.content_type).await
                },
            ))
            .await;
            if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
                return Err(ApiError::msg(format!(
                    "Uploading a page snapshot failed: {e}"
                )));
            }
            db::insert("questions", &rows).await?;
        }
    }

    db::update(
        "certifications",
        &format!("id=eq.{cert_id}"),
        &json!({ "question_count": existing_count as usize + total }),
    )
    .await?;

    if opts.queue_explanations {
        stage.set(Stage::Importing {
            label: "Queueing Claude explanations…".into(),
            done: total,
            total,
        });
        data::request_explanations(&all_ids, false).await?;
    }
    Ok((cert_id, total))
}
