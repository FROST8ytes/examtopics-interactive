//! Runs the ExamTopics parser on extracted page text and prints a summary.
//!
//! The input is a JSON array of `RawPage` (what pdf.js produces in the browser):
//!
//! ```sh
//! cargo run -p examcore --example parse_json -- pages.json [--verbose]
//! ```

use examcore::layout::RawPage;
use examcore::parse::{parse_pages, plan_sets};
use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: parse_json <pages.json> [--verbose]");
    let verbose = args.any(|a| a == "--verbose");
    let pages: Vec<RawPage> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read input"))
            .expect("parse JSON");
    let out = parse_pages(&pages);

    println!("pages: {}  questions: {}", pages.len(), out.questions.len());
    println!("title: {:?}  exam code: {:?}", out.title, out.exam_code);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_topic: BTreeMap<u32, usize> = BTreeMap::new();
    for q in &out.questions {
        *kinds.entry(q.question.kind.label()).or_default() += 1;
        *per_topic.entry(q.question.topic).or_default() += 1;
    }
    println!("kinds: {kinds:?}");
    for t in &out.topics {
        println!(
            "  topic {:>2} {:<28} {:>3} questions",
            t.number,
            t.title,
            per_topic.get(&t.number).unwrap_or(&0)
        );
    }
    let warned: Vec<_> = out
        .questions
        .iter()
        .filter(|q| !q.warnings.is_empty())
        .collect();
    println!("questions with warnings: {}", warned.len());
    for q in warned.iter().take(if verbose { usize::MAX } else { 15 }) {
        println!(
            "  T{} #{}: {}",
            q.question.topic,
            q.question.number,
            q.warnings.join(" | ")
        );
    }
    println!("sets (50 per set):");
    for p in plan_sets(&out, 50) {
        println!("  {:<45} {:>3}", p.title, p.end - p.start);
    }
    if verbose {
        for q in &out.questions {
            println!(
                "--- T{} #{} [{}] answer={}\n{}",
                q.question.topic,
                q.question.number,
                q.question.kind.label(),
                q.question.kind.answer_summary().replace('\n', " / "),
                q.question.body.chars().take(160).collect::<String>()
            );
        }
    }
}
