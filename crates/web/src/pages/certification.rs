use crate::Route;
use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonSize, ButtonVariant};
use crate::components::card::*;
use crate::data::{self, Certification, SetStats};
use crate::supabase::db;
use crate::ui::{ErrorState, Loading, Meter, percent};
use dioxus::prelude::*;
use dioxus_icons::lucide::{Play, RotateCcw, Sparkles, Trash2};
use dioxus_primitives::toast::{ToastOptions, use_toast};
use serde::Deserialize;

#[derive(Clone, PartialEq)]
struct Loaded {
    cert: Certification,
    stats: Vec<SetStats>,
    can_edit: bool,
    can_explain: bool,
}

async fn load(id: String) -> Result<Loaded, String> {
    let user_id = crate::supabase::auth::SESSION
        .peek()
        .as_ref()
        .map(|s| s.user.id.clone())
        .unwrap_or_default();
    let (cert, stats, can_edit, profile) = futures_util::join!(
        data::certification(&id),
        data::my_stats(Some(&id)),
        data::can_edit(&id),
        data::profile(&user_id)
    );
    Ok(Loaded {
        cert: cert
            .map_err(|e| e.to_string())?
            .ok_or("Certification not found")?,
        stats: stats.map_err(|e| e.to_string())?,
        can_edit,
        can_explain: profile.ok().flatten().is_some_and(|p| p.can_use_explainer),
    })
}

pub fn confirm(message: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(message).ok())
        .unwrap_or(false)
}

#[component]
pub fn CertificationPage(id: String) -> Element {
    let mut loaded = use_resource(use_reactive!(|id| load(id)));
    let nav = use_navigator();
    let toast = use_toast();
    let mut busy = use_signal(|| false);

    let l = match &*loaded.read() {
        None => return rsx! { Loading {} },
        Some(Err(e)) => {
            return rsx! { ErrorState { message: e.clone(), on_retry: move |_| loaded.restart() } };
        }
        Some(Ok(l)) => l.clone(),
    };
    let total: i32 = l.stats.iter().map(|s| s.question_count).sum();
    let answered: i32 = l.stats.iter().map(|s| s.answered).sum();
    let correct: i32 = l.stats.iter().map(|s| s.correct).sum();
    let cert_id = l.cert.id.clone();

    let queue_all = {
        let cert_id = cert_id.clone();
        move |_| {
            let cert_id = cert_id.clone();
            async move {
                #[derive(Deserialize)]
                struct Id {
                    id: String,
                }
                busy.set(true);
                let ids: Result<Vec<Id>, _> = db::select(
                    "questions",
                    &format!("select=id&certification_id=eq.{cert_id}"),
                )
                .await;
                let result = match ids {
                    Ok(ids) => {
                        data::request_explanations(
                            &ids.into_iter().map(|i| i.id).collect::<Vec<_>>(),
                            false,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                };
                busy.set(false);
                match result {
                    Ok(n) => toast.success(
                        format!("Queued {n} explanations"),
                        ToastOptions::new()
                            .description("The explainer worker processes them in the background."),
                    ),
                    Err(e) => toast.error(
                        "Couldn't queue explanations".into(),
                        ToastOptions::new().description(e.to_string()),
                    ),
                }
            }
        }
    };

    let delete = {
        let cert = l.cert.clone();
        move |_| {
            let cert = cert.clone();
            async move {
                if !confirm(&format!(
                    "Delete {} and all its sets, questions and everyone's progress? This cannot be undone.",
                    cert.code
                )) {
                    return;
                }
                busy.set(true);
                match data::delete_certification(&cert.id).await {
                    Ok(()) => {
                        nav.replace(Route::Dashboard {});
                    }
                    Err(e) => {
                        busy.set(false);
                        toast.error(
                            "Couldn't delete".into(),
                            ToastOptions::new().description(e.to_string()),
                        );
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "container stack-lg",
            div { class: "page-head",
                div {
                    div { class: "row", style: "gap:.5rem;margin-bottom:.35rem",
                        Link { class: "link small", to: Route::Dashboard {}, "Library" }
                        span { class: "muted small", "/" }
                        Badge { variant: BadgeVariant::Secondary, "{l.cert.code}" }
                    }
                    h1 { "{l.cert.name}" }
                    p { class: "muted",
                        "{l.stats.len()} sets · {total} questions"
                        if answered > 0 { " · {percent(answered, total)}% done · {percent(correct, answered)}% correct" }
                    }
                }
                if l.can_edit {
                    div { class: "row",
                        if l.can_explain {
                            Button { variant: ButtonVariant::Outline, disabled: busy(), onclick: queue_all,
                                Sparkles { size: "16px" } "Explain all with Claude"
                            }
                        }
                        Button { variant: ButtonVariant::Destructive, disabled: busy(), onclick: delete,
                            Trash2 { size: "16px" } "Delete"
                        }
                    }
                }
            }
            if let Some(d) = &l.cert.description {
                p { class: "muted", "{d}" }
            }
            Meter { total, answered, correct }
            div { class: "stack",
                for s in l.stats.iter() {
                    SetCard { key: "{s.set_id}", stats: s.clone(), on_reset: move |_| loaded.restart() }
                }
            }
        }
    }
}

#[component]
fn SetCard(stats: SetStats, on_reset: EventHandler<()>) -> Element {
    let toast = use_toast();
    let finished = stats.answered >= stats.question_count && stats.question_count > 0;
    let started = stats.answered > 0 || stats.current_position.is_some();
    let resume = stats
        .current_position
        .unwrap_or(0)
        .clamp(0, (stats.question_count - 1).max(0))
        + 1;
    let set_id = stats.set_id.clone();
    rsx! {
        Card {
            CardHeader {
                CardTitle { "{stats.title}" }
                CardDescription {
                    "{stats.question_count} questions"
                    if started { " · {stats.answered} answered · {percent(stats.correct, stats.answered)}% correct" }
                }
                CardAction {
                    div { class: "row", style: "gap:.4rem",
                        if started {
                            Button {
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Ghost,
                                title: "Reset progress",
                                onclick: move |_| {
                                    let set_id = set_id.clone();
                                    async move {
                                        if !super::certification::confirm("Reset your progress for this set?") { return; }
                                        match data::reset_set(&set_id).await {
                                            Ok(()) => on_reset.call(()),
                                            Err(e) => toast.error("Couldn't reset".into(), ToastOptions::new().description(e.to_string())),
                                        }
                                    }
                                },
                                RotateCcw { size: "14px" }
                            }
                        }
                        Link {
                            to: Route::Quiz { id: stats.set_id.clone(), q: if finished { "1".into() } else { resume.to_string() } },
                            Button { size: ButtonSize::Sm, variant: if started { ButtonVariant::Primary } else { ButtonVariant::Outline },
                                Play { size: "14px" }
                                if finished { "Review" } else if started { "Continue at Q{resume}" } else { "Start" }
                            }
                        }
                    }
                }
            }
            CardContent {
                Meter { total: stats.question_count, answered: stats.answered, correct: stats.correct }
            }
        }
    }
}
