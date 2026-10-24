//! Typed rows and queries for the app tables (see supabase/migrations).

use crate::supabase::{Result, db, storage};
use examcore::{Explanation, Question, QuestionKind, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Certification {
    pub id: String,
    pub code: String,
    pub name: String,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub question_count: i32,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExamSet {
    pub id: String,
    pub certification_id: String,
    pub title: String,
    pub topic: i32,
    pub intro: Option<String>,
    pub position: i32,
    pub question_count: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QuestionRow {
    pub id: String,
    pub set_id: String,
    pub certification_id: String,
    pub position: i32,
    pub number: i32,
    pub topic: i32,
    pub data: Question,
    pub source: Question,
    #[serde(default)]
    pub parse_warnings: Vec<String>,
    pub edited_at: Option<String>,
}

impl QuestionRow {
    /// The answer key was changed after import (OCR fix or corrected key).
    pub fn key_edited(&self) -> bool {
        self.data.kind != self.source.kind
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SetStats {
    pub set_id: String,
    pub certification_id: String,
    pub title: String,
    pub topic: i32,
    pub position: i32,
    pub question_count: i32,
    pub answered: i32,
    pub correct: i32,
    pub current_position: Option<i32>,
    pub last_activity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProgressRow {
    pub question_id: String,
    pub response: Response,
    pub is_correct: bool,
    pub attempts: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExplanationRow {
    pub question_id: String,
    pub content: Explanation,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JobRow {
    pub question_id: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Profile {
    pub id: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
    /// May queue explanations for the owner's Claude subscription.
    #[serde(default)]
    pub can_use_explainer: bool,
}

// ---- library -----------------------------------------------------------------

pub async fn certifications() -> Result<Vec<Certification>> {
    db::select("certifications", "select=*&order=code.asc,created_at.asc").await
}

pub async fn certification(id: &str) -> Result<Option<Certification>> {
    db::select_one("certifications", &format!("select=*&id=eq.{id}")).await
}

pub async fn my_stats(certification_id: Option<&str>) -> Result<Vec<SetStats>> {
    let filter = certification_id
        .map(|c| format!("&certification_id=eq.{c}"))
        .unwrap_or_default();
    db::select(
        "my_set_stats",
        &format!("select=*{filter}&order=position.asc"),
    )
    .await
}

pub async fn exam_set(id: &str) -> Result<Option<ExamSet>> {
    db::select_one("exam_sets", &format!("select=*&id=eq.{id}")).await
}

pub async fn questions(set_id: &str) -> Result<Vec<QuestionRow>> {
    db::select(
        "questions",
        &format!(
            "select=id,set_id,certification_id,position,number,topic,data,source,parse_warnings,edited_at&set_id=eq.{set_id}&order=position.asc"
        ),
    )
    .await
}

pub async fn question(id: &str) -> Result<Option<QuestionRow>> {
    db::select_one(
        "questions",
        &format!("select=id,set_id,certification_id,position,number,topic,data,source,parse_warnings,edited_at&id=eq.{id}"),
    )
    .await
}

pub async fn profile(user_id: &str) -> Result<Option<Profile>> {
    db::select_one(
        "profiles",
        &format!("select=id,display_name,is_admin,can_use_explainer&id=eq.{user_id}"),
    )
    .await
}

pub async fn can_edit(certification_id: &str) -> bool {
    db::rpc::<bool>(
        "can_edit_certification",
        &json!({ "p_certification_id": certification_id }),
    )
    .await
    .unwrap_or(false)
}

// ---- progress ----------------------------------------------------------------

pub async fn progress(set_id: &str) -> Result<Vec<ProgressRow>> {
    db::select(
        "question_progress",
        &format!("select=question_id,response,is_correct,attempts&set_id=eq.{set_id}"),
    )
    .await
}

pub async fn record_answer(
    question_id: &str,
    response: &Response,
    is_correct: bool,
    next_position: i32,
) -> Result<()> {
    db::rpc::<serde_json::Value>(
        "record_answer",
        &json!({
            "p_question_id": question_id,
            "p_response": response,
            "p_is_correct": is_correct,
            "p_next_position": next_position,
        }),
    )
    .await
    .map(|_| ())
}

pub async fn save_position(set_id: &str, position: i32) -> Result<()> {
    db::rpc::<serde_json::Value>(
        "save_position",
        &json!({ "p_set_id": set_id, "p_position": position }),
    )
    .await
    .map(|_| ())
}

pub async fn reset_set(set_id: &str) -> Result<()> {
    db::rpc::<serde_json::Value>("reset_set_progress", &json!({ "p_set_id": set_id }))
        .await
        .map(|_| ())
}

// ---- explanations --------------------------------------------------------------

pub async fn explanations(question_ids: &[String]) -> Result<HashMap<String, ExplanationRow>> {
    let mut out = HashMap::new();
    for chunk in question_ids.chunks(80) {
        let rows: Vec<ExplanationRow> = db::select(
            "explanations",
            &format!(
                "select=question_id,content,model&question_id={}",
                db::in_list(chunk.iter().map(String::as_str))
            ),
        )
        .await?;
        out.extend(rows.into_iter().map(|r| (r.question_id.clone(), r)));
    }
    Ok(out)
}

/// Latest job per question (pending / processing / failed).
pub async fn jobs(question_ids: &[String]) -> Result<HashMap<String, JobRow>> {
    let mut out = HashMap::new();
    for chunk in question_ids.chunks(80) {
        let rows: Vec<JobRow> = db::select(
            "ai_jobs",
            &format!(
                "select=question_id,status,error&status=neq.done&order=created_at.asc&question_id={}",
                db::in_list(chunk.iter().map(String::as_str))
            ),
        )
        .await?;
        out.extend(rows.into_iter().map(|r| (r.question_id.clone(), r)));
    }
    Ok(out)
}

pub async fn request_explanations(question_ids: &[String], force: bool) -> Result<i64> {
    let mut total = 0;
    for chunk in question_ids.chunks(1000) {
        total += db::rpc::<i64>(
            "request_explanations",
            &json!({ "p_question_ids": chunk, "p_force": force }),
        )
        .await?;
    }
    Ok(total)
}

// ---- editing ----------------------------------------------------------------------

#[derive(Serialize)]
struct QuestionUpdate<'a> {
    data: &'a Question,
    edited_by: &'a str,
    edited_at: String,
}

pub async fn save_question(id: &str, data: &Question, user_id: &str) -> Result<()> {
    let now = String::from(js_sys::Date::new_0().to_iso_string());
    db::update(
        "questions",
        &format!("id=eq.{id}"),
        &QuestionUpdate {
            data,
            edited_by: user_id,
            edited_at: now,
        },
    )
    .await
}

/// Applies Claude's suggested letters to a choice question.
pub fn with_choice_answer(q: &Question, letters: &str) -> Option<Question> {
    let QuestionKind::Choice { options, .. } = &q.kind else {
        return None;
    };
    let correct: Vec<String> = letters
        .chars()
        .filter(|c| c.is_ascii_uppercase())
        .map(|c| c.to_string())
        .filter(|k| options.iter().any(|o| &o.key == k))
        .collect();
    if correct.is_empty() {
        return None;
    }
    let mut q = q.clone();
    q.kind = QuestionKind::Choice {
        options: options.clone(),
        correct,
    };
    Some(q)
}

pub async fn delete_certification(id: &str) -> Result<()> {
    #[derive(Deserialize)]
    struct Imgs {
        data: Question,
    }
    let rows: Vec<Imgs> = db::select(
        "questions",
        &format!("select=data&certification_id=eq.{id}"),
    )
    .await?;
    let paths: Vec<String> = rows
        .into_iter()
        .flat_map(|r| [r.data.images.question, r.data.images.answer])
        .flatten()
        .collect();
    for chunk in paths.chunks(500) {
        let _ = storage::remove(chunk).await;
    }
    db::delete("certifications", &format!("id=eq.{id}")).await
}

/// Signed URLs for every snapshot of the given questions.
pub async fn image_urls(questions: &[QuestionRow]) -> HashMap<String, String> {
    let paths: Vec<String> = questions
        .iter()
        .flat_map(|q| [q.data.images.question.clone(), q.data.images.answer.clone()])
        .flatten()
        .collect();
    storage::signed_urls(&paths, 60 * 60 * 6)
        .await
        .unwrap_or_default()
}
