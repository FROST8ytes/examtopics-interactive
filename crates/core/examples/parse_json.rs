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
    let rest: Vec<String> = args.collect();
    let verbose = rest.iter().any(|a| a == "--verbose");
    let pages: Vec<RawPage> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read input"))
            .expect("parse JSON");
    // `--lines N`: print the lines built for page N (1-based) and exit.
    if let Some(n) = rest
        .iter()
        .position(|a| a == "--lines")
        .and_then(|i| rest.get(i + 1)?.parse::<usize>().ok())
    {
        let lines = examcore::layout::build_lines(&pages[n - 1..n]);
        for l in lines {
            let spans: Vec<String> = l
                .spans
                .iter()
                .map(|s| format!("[{:.0}-{:.0} {}]", s.left, s.right, s.text))
                .collect();
            println!(
                "{:6.1}-{:6.1} x{:5.0}-{:5.0} | {}",
                l.top,
                l.bottom,
                l.left,
                l.right,
                spans.join(" ")
            );
        }
        return;
    }
    let out = parse_pages(&pages);
    // `--pictures`: list the answer pictures of dropdown questions (to render them).
    if rest.iter().any(|a| a == "--pictures") {
        let list: Vec<serde_json::Value> = out
            .questions
            .iter()
            .filter(|q| matches!(q.question.kind, examcore::QuestionKind::Dropdown { .. }))
            .filter_map(|q| {
                Some(serde_json::json!({
                    "key": format!("T{}-{}", q.question.topic, q.question.number),
                    "answer": q.answer_picture?,
                    "question": q.question_picture,
                }))
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list).unwrap());
        return;
    }
    // `--marks DIR`: read the keys of dropdown questions from pictures rendered into
    // DIR as `<key>-a.rgba` / `<key>-q.rgba` (raw RGBA, sizes in `sizes.json`).
    if let Some(dir) = rest
        .iter()
        .position(|a| a == "--marks")
        .and_then(|i| rest.get(i + 1))
    {
        let sizes: std::collections::HashMap<String, (usize, usize)> = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/sizes.json")).expect("sizes.json"),
        )
        .unwrap();
        let load = |name: &str| -> Option<(Vec<u8>, usize, usize)> {
            let (w, h) = *sizes.get(name)?;
            Some((std::fs::read(format!("{dir}/{name}.rgba")).ok()?, w, h))
        };
        let (mut ok, mut total) = (0, 0);
        for q in &out.questions {
            let examcore::QuestionKind::Dropdown { groups } = &q.question.kind else {
                continue;
            };
            let key = format!("T{}-{}", q.question.topic, q.question.number);
            let Some((a, aw, ah)) = load(&format!("{key}-a")) else {
                continue;
            };
            total += 1;
            let qimg = load(&format!("{key}-q"));
            let answer = examcore::marks::Image {
                width: aw,
                height: ah,
                rgba: &a,
            };
            let question = qimg.as_ref().map(|(b, w, h)| examcore::marks::Image {
                width: *w,
                height: *h,
                rgba: b,
            });
            let marks = examcore::marks::find_marks(&answer, question.as_ref());
            let mut keyed = groups.clone();
            let done = examcore::marks::key_dropdowns(&mut keyed, &answer, question.as_ref());
            ok += done as usize;
            let rows: Vec<String> = marks
                .iter()
                .map(|m| {
                    format!(
                        "{:?}@({},{} {}x{})",
                        examcore::marks::list_position(&answer, *m),
                        m.x0,
                        m.y0,
                        m.x1 - m.x0,
                        m.y1 - m.y0
                    )
                })
                .collect();
            let counts: Vec<usize> = groups.iter().map(|g| g.options.len()).collect();
            let text_key: Vec<Option<usize>> = groups.iter().map(|g| g.correct).collect();
            println!(
                "{key}: options={counts:?} marks={} rows={rows:?} diff={} {}{}",
                marks.len(),
                question.is_some(),
                if done { "OK" } else { "--" },
                if text_key.iter().any(Option::is_some) {
                    format!(" text={text_key:?}")
                } else {
                    String::new()
                }
            );
            if done {
                for g in &keyed {
                    println!("    {} → {}", g.label, g.options[g.correct.unwrap()]);
                }
            }
        }
        println!("keyed {ok}/{total}");
        return;
    }
    // `--visual`: print the lines of every image-based / hotspot / drag & drop question.
    if rest.iter().any(|a| a == "--visual") {
        let lines = examcore::layout::build_lines(&pages);
        for q in &out.questions {
            if matches!(q.question.kind, examcore::QuestionKind::Choice { .. }) {
                continue;
            }
            let state = match &q.question.kind {
                examcore::QuestionKind::Visual { .. } => "IMAGE",
                k if k.has_key() => "KEYED",
                _ => "NOKEY",
            };
            println!(
                "\n##### T{} #{} [{}] {state} exhibit={} answer={}",
                q.question.topic,
                q.question.number,
                q.question.kind.label(),
                q.question.has_exhibit,
                q.question.kind.answer_summary().replace('\n', " / ")
            );
            match &q.question.kind {
                examcore::QuestionKind::Dropdown { groups } => {
                    for g in groups {
                        println!("  ▸ {} {:?} = {:?}", g.label, g.options, g.correct);
                    }
                }
                examcore::QuestionKind::DragDrop { items, slots, .. } => {
                    println!("  ▸ items {items:?}");
                    for s in slots {
                        println!("  ▸ slot {} = {:?}", s.label, s.correct);
                    }
                }
                _ => {}
            }
            if let Some(p) = q.answer_picture {
                println!("  answer picture p{} {:?}", p.page + 1, p.rect);
            }
            for (name, regions) in [("Q", &q.question_regions), ("A", &q.answer_regions)] {
                for r in regions.iter() {
                    println!("  [{name} p{} {:.0}-{:.0}]", r.page + 1, r.top, r.bottom);
                    for l in lines
                        .iter()
                        .filter(|l| l.page == r.page && l.top >= r.top && l.top < r.bottom)
                    {
                        println!(
                            "    {:6.1}-{:6.1} x{:4.0}-{:4.0} | {}",
                            l.top, l.bottom, l.left, l.right, l.text
                        );
                    }
                }
            }
        }
        return;
    }

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
