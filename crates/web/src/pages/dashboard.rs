use crate::Route;
use crate::components::badge::{Badge, BadgeVariant};
use crate::components::button::{Button, ButtonVariant};
use crate::components::card::*;
use crate::components::skeleton::Skeleton;
use crate::data::{self, Certification, SetStats};
use crate::supabase::auth::SESSION;
use crate::ui::{ErrorState, Meter, percent};
use dioxus::prelude::*;
use dioxus_icons::lucide::{BookOpen, Play, Upload};

#[derive(Clone, PartialEq)]
struct Library {
    certs: Vec<Certification>,
    stats: Vec<SetStats>,
}

#[component]
pub fn Dashboard() -> Element {
    let mut library = use_resource(|| async {
        let (certs, stats) =
            futures_util::future::join(data::certifications(), data::my_stats(None)).await;
        Ok::<_, String>(Library {
            certs: certs.map_err(|e| e.to_string())?,
            stats: stats.map_err(|e| e.to_string())?,
        })
    });
    let is_guest = SESSION.read().as_ref().is_some_and(|s| s.user.is_anonymous);

    let content = match &*library.read() {
        None => rsx! {
            div { class: "grid",
                for _ in 0..3 { Skeleton { style: "height:9rem;border-radius:.75rem" } }
            }
        },
        Some(Err(e)) => {
            rsx! { ErrorState { message: e.clone(), on_retry: move |_| library.restart() } }
        }
        Some(Ok(lib)) => {
            // Most recent activity → "continue where you left off".
            let resume = lib
                .stats
                .iter()
                .filter(|s| s.last_activity.is_some())
                .max_by(|a, b| a.last_activity.cmp(&b.last_activity))
                .cloned();
            rsx! {
                if let Some(r) = resume {
                    ResumeCard { stats: r.clone(), cert: lib.certs.iter().find(|c| c.id == r.certification_id).cloned() }
                }
                if lib.certs.is_empty() {
                    Card {
                        CardContent {
                            div { class: "center", style: "min-height:16rem",
                                div { class: "stack", style: "align-items:center",
                                    BookOpen { size: "32px" }
                                    h3 { "No certifications yet" }
                                    p { class: "muted small",
                                        if is_guest { "Ask a teammate to upload an ExamTopics PDF, or sign in to upload one yourself." }
                                        else { "Upload an ExamTopics PDF to turn it into an interactive quiz." }
                                    }
                                    if !is_guest {
                                        Link { to: Route::Upload {},
                                            Button { Upload { size: "16px" } "Upload PDF" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    div { class: "stack-sm",
                        h2 { "Certifications" }
                        div { class: "grid",
                            for cert in lib.certs.iter() {
                                CertCard {
                                    key: "{cert.id}",
                                    cert: cert.clone(),
                                    stats: lib.stats.iter().filter(|s| s.certification_id == cert.id).cloned().collect::<Vec<_>>(),
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "container stack-lg",
            div { class: "page-head",
                div {
                    h1 { "Library" }
                    p { class: "muted", "Practice ExamTopics questions set by set. Your progress is saved automatically." }
                }
                if !is_guest {
                    Link { to: Route::Upload {},
                        Button { variant: ButtonVariant::Outline, Upload { size: "16px" } "Upload PDF" }
                    }
                }
            }
            {content}
        }
    }
}

#[component]
fn ResumeCard(stats: SetStats, cert: Option<Certification>) -> Element {
    let position = stats
        .current_position
        .unwrap_or(0)
        .min(stats.question_count - 1)
        .max(0);
    let finished = stats.answered >= stats.question_count;
    rsx! {
        Card { style: "border-color: var(--focused-border-color)",
            CardHeader {
                CardDescription { "Continue where you left off" }
                CardTitle {
                    if let Some(c) = &cert { "{c.code} · " }
                    "{stats.title}"
                }
                CardAction {
                    Link { to: Route::Quiz { id: stats.set_id.clone(), q: (position + 1).to_string() },
                        Button { Play { size: "16px" } if finished { "Review" } else { "Resume at Q{position + 1}" } }
                    }
                }
            }
            CardContent {
                div { class: "stack-sm",
                    Meter { total: stats.question_count, answered: stats.answered, correct: stats.correct }
                    span { class: "small muted",
                        "{stats.answered}/{stats.question_count} answered · {percent(stats.correct, stats.answered)}% correct"
                    }
                }
            }
        }
    }
}

#[component]
fn CertCard(cert: Certification, stats: Vec<SetStats>) -> Element {
    let total: i32 = stats.iter().map(|s| s.question_count).sum();
    let answered: i32 = stats.iter().map(|s| s.answered).sum();
    let correct: i32 = stats.iter().map(|s| s.correct).sum();
    rsx! {
        Link { class: "card-link", to: Route::CertificationPage { id: cert.id.clone() },
            Card { style: "height:100%",
                CardHeader {
                    div { class: "row", style: "gap:.4rem",
                        Badge { variant: BadgeVariant::Secondary, "{cert.code}" }
                        if let Some(v) = &cert.vendor { span { class: "tiny muted", "{v}" } }
                    }
                    CardTitle { "{cert.name}" }
                    CardDescription { "{stats.len()} sets · {total} questions" }
                }
                CardContent {
                    div { class: "stack-sm",
                        Meter { total, answered, correct }
                        span { class: "small muted",
                            if answered == 0 { "Not started" } else { "{percent(answered, total)}% done · {percent(correct, answered)}% correct" }
                        }
                    }
                }
            }
        }
    }
}
