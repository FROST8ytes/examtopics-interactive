use examcore::layout::{RawItem, RawPage};
use examcore::parse::{parse_pages, plan_sets};
use examcore::{QuestionKind, VisualVariant};

/// Lays out text as an OCR text layer would: one item per line, `---page---`
/// starts a new page, `[gap N]` leaves N points of empty space (a picture).
fn pages(text: &str) -> Vec<RawPage> {
    let mut pages = Vec::new();
    for (pi, chunk) in text.split("---page---").enumerate() {
        let mut items = vec![
            RawItem {
                s: "ExamTopics PDF Export".into(),
                x: 40.0,
                y: 15.0,
                w: 120.0,
                h: 9.0,
            },
            RawItem {
                s: format!("{}", pi + 1),
                x: 290.0,
                y: 820.0,
                w: 10.0,
                h: 9.0,
            },
        ];
        let mut y = 70.0;
        for line in chunk
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
        {
            if let Some(gap) = line
                .trim()
                .strip_prefix("[gap ")
                .and_then(|g| g.strip_suffix(']'))
            {
                y += gap.parse::<f32>().unwrap();
                continue;
            }
            // Right aligned "Topic N" next to the question header, like the real PDF.
            if let Some((left, right)) = line.split_once("    ") {
                items.push(RawItem {
                    s: left.trim().into(),
                    x: 40.0,
                    y,
                    w: left.len() as f32 * 5.0,
                    h: 11.0,
                });
                items.push(RawItem {
                    s: right.trim().into(),
                    x: 480.0,
                    y: y + 0.5,
                    w: 40.0,
                    h: 11.0,
                });
            } else {
                items.push(RawItem {
                    s: line.trim().into(),
                    x: 40.0,
                    y,
                    w: line.len() as f32 * 5.0,
                    h: 11.0,
                });
            }
            y += 15.0;
        }
        pages.push(RawPage {
            width: 595.0,
            height: 842.0,
            items,
        });
    }
    pages
}

const SAMPLE: &str = r#"
Microsoft AZ-104 Exam
Microsoft Azure Administrator
Topic 1 - Question Set 1
Question #1    Topic 1
You have an Azure subscription named Subscription1.
You need to ensure that users can create resource groups.
What should you use?
A. a management group
B. an Azure policy that spans
multiple lines of text
C. a resource lock
D. role-based access control (RBAC)
Correct Answer: D
Community vote distribution
D (91%) 9%
Question #2    Topic 1
You plan to deploy three virtual machines.
Which two actions should you perform? Each correct answer presents part of the solution.
NOTE: Each correct selection is worth one point.
A. Create an availability set.
B. Create a scale set.
C. Configure a load balancer.
D. Configure network peering.
Correct Answer: AC
Explanation:
Availability sets protect against rack failures.
Reference:
https://learn.microsoft.com/azure/virtual-machines/availability
Community vote distribution
AC (70%) AB (30%)
---page---
Question #3    Topic 1
HOTSPOT -
You have the following resources.
For each of the following statements, select Yes if the statement is true. Otherwise, select No.
NOTE: Each correct selection is worth one point.
Hot Area:
Statements Yes No
VM1 can connect to VM2. Yes No
VM2 can connect to the internet. Yes No
Correct Answer:
Box 1: Yes
Box 2: No
Question #4    Topic 1
DRAG DROP -
You need to move a VM to another region.
Which three actions should you perform in sequence? To answer, move the appropriate actions to the answer area and arrange them in the correct order.
Select and Place:
[gap 160]
Correct Answer:
[gap 120]
Question #5    Topic 1
You have the network shown in the exhibit.
[gap 200]
Does this meet the goal?
A. Yes
B. No
Correct Answer: B
---page---
Topic 2 - Contoso, Ltd. Case Study
Question #1    Topic 2
Introductory Info
Case study -
Contoso, Ltd. is a consulting company.
Question
You need to identify the storage requirements for Contoso.
What should you use?
A. Azure Files
B. Azure Blob storage
Correct Answer: Box 1
Community vote distribution
B (80%) A (20%)
"#;

#[test]
fn parses_sample_document() {
    let out = parse_pages(&pages(SAMPLE));
    assert_eq!(out.exam_code.as_deref(), Some("AZ-104"));
    assert_eq!(
        out.questions.len(),
        6,
        "{:#?}",
        out.questions
            .iter()
            .map(|q| &q.question.body)
            .collect::<Vec<_>>()
    );
    assert_eq!(out.topics.len(), 2);
    assert_eq!(out.topics[0].title, "Question Set 1");
    assert_eq!(out.topics[1].title, "Contoso, Ltd. Case Study");

    // Q1: single choice, wrapped option, community votes.
    let q1 = &out.questions[0].question;
    assert_eq!((q1.number, q1.topic), (1, 1));
    assert!(
        q1.body
            .starts_with("You have an Azure subscription named Subscription1.")
    );
    assert!(!q1.body.contains("ExamTopics PDF Export"));
    match &q1.kind {
        QuestionKind::Choice { options, correct } => {
            assert_eq!(options.len(), 4);
            assert_eq!(
                options[1].text,
                "an Azure policy that spans multiple lines of text"
            );
            assert_eq!(correct, &vec!["D".to_string()]);
        }
        k => panic!("unexpected {k:?}"),
    }
    assert_eq!(q1.community_votes.len(), 1);
    assert_eq!(q1.community_votes[0].percent, 91);
    assert!(!q1.has_exhibit);

    // Q2: multiple choice + explanation + reference.
    let q2 = &out.questions[1];
    match &q2.question.kind {
        QuestionKind::Choice { correct, .. } => {
            assert_eq!(correct, &vec!["A".to_string(), "C".to_string()])
        }
        k => panic!("unexpected {k:?}"),
    }
    assert_eq!(
        q2.question.source_explanation.as_deref(),
        Some("Availability sets protect against rack failures.")
    );
    assert_eq!(
        q2.question.references,
        vec!["https://learn.microsoft.com/azure/virtual-machines/availability"]
    );
    assert!(q2.warnings.is_empty(), "{:?}", q2.warnings);

    // Q3: Yes/No hotspot rebuilt as dropdowns.
    match &out.questions[2].question.kind {
        QuestionKind::Dropdown { groups } => {
            assert_eq!(groups.len(), 2);
            assert_eq!(groups[0].label, "VM1 can connect to VM2.");
            assert_eq!(groups[0].correct, 0);
            assert_eq!(groups[1].correct, 1);
        }
        k => panic!("unexpected {k:?}"),
    }

    // Q4: drag & drop stays visual, with regions that include the pictures.
    let q4 = &out.questions[3];
    assert!(matches!(
        q4.question.kind,
        QuestionKind::Visual {
            variant: VisualVariant::DragDrop,
            ..
        }
    ));
    assert!(q4.question.has_exhibit);
    let qr = q4.question_regions[0];
    assert!(qr.bottom - qr.top > 160.0, "{qr:?}");
    let ar = q4.answer_regions[0];
    assert!(ar.bottom - ar.top > 120.0, "{ar:?}");

    // Q5: exhibit detected from the empty band.
    let q5 = &out.questions[4].question;
    assert!(q5.has_exhibit);
    assert!(matches!(&q5.kind, QuestionKind::Choice { options, .. } if options.len() == 2));

    // Case study question: intro split out, unreadable key falls back to votes.
    let cs = &out.questions[5];
    assert_eq!(cs.question.topic, 2);
    assert_eq!(
        cs.question.case_study.as_deref(),
        Some("Case study -\nContoso, Ltd. is a consulting company.")
    );
    assert!(cs.question.body.starts_with("You need to identify"));
    match &cs.question.kind {
        QuestionKind::Choice { correct, .. } => assert_eq!(correct, &vec!["B".to_string()]),
        k => panic!("unexpected {k:?}"),
    }
    assert_eq!(cs.warnings.len(), 1);
}

#[test]
fn question_spanning_pages_gets_two_regions() {
    let text = "Question #7    Topic 1\nLine one\nLine two\n---page---\nA. one\nB. two\nCorrect Answer: A\nQuestion #8    Topic 1\nX\nA. a\nB. b\nCorrect Answer: B";
    let out = parse_pages(&pages(text));
    assert_eq!(out.questions.len(), 2);
    let r = &out.questions[0].question_regions;
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].page, r[1].page), (0, 1));
}

#[test]
fn body_mentions_of_question_are_not_headers() {
    let text = "Question #1    Topic 1\nAnswer Question 3 of the case study first.\nA. x\nB. y\nCorrect Answer: A";
    let out = parse_pages(&pages(text));
    assert_eq!(out.questions.len(), 1);
    assert!(
        out.questions[0]
            .question
            .body
            .contains("Question 3 of the case study")
    );
}

#[test]
fn plans_sets_by_topic_and_chunk() {
    let out = parse_pages(&pages(SAMPLE));
    let plans = plan_sets(&out, 2);
    let titles: Vec<&str> = plans.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(plans.len(), 4, "{titles:?}");
    assert_eq!(titles[0], "Topic 1 – Question Set 1 · Set 1 (Q1–2)");
    assert_eq!(titles[3], "Topic 2 – Contoso, Ltd. Case Study");
    assert_eq!(plan_sets(&out, 0).len(), 2);
}

/// Patterns seen in a real OCR'd SC-100 export: topic misread in headers ("Topic 7" for
/// "Topic 1"), headers lost entirely (with or without their "Correct Answer" line) and
/// picture text glued into a header row.
const OCR_NOISE: &str = r#"
Topic 1 - Question Set 1
Question #1    Topic 7
You need to recommend a privacy solution.
A. Microsoft Priva
B. Microsoft Viva
Correct Answer: A
Community vote distribution
A (100%)
HOTSPOT -
You have an Azure subscription.
Hot Area:
[gap 80]
Question #3    Topic 17
You have a hotspot whose answer is only a picture.
HOTSPOT -
Correct Answer: B
Box 1: Azure Purview
Note: This question is part of a series of questions that present the same scenario.
Solution: You enable MFA.
Does this meet the goal?
A. Yes
B. No
Correct Answer: B
---page---
Topic 2 - Question Set 2
Question #1 For the inbound connections: v    Topic 2
You have two virtual networks.
A. Peering
B. VPN
Correct Answer: A
"#;

#[test]
fn recovers_from_ocr_noise() {
    let out = parse_pages(&pages(OCR_NOISE));
    let summary: Vec<(u32, u32, &str)> = out
        .questions
        .iter()
        .map(|q| (q.question.topic, q.question.number, q.question.kind.label()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (1, 1, "Single choice"),
            // Header and answer key both lost: recovered after the community vote.
            (1, 2, "Hotspot"),
            (1, 3, "Hotspot"),
            // Header lost, found from the "Note: This question is part of a series" opener.
            (1, 4, "Single choice"),
            (2, 1, "Single choice"),
        ],
        "{summary:?}"
    );
    assert!(out.questions[1].warnings[0].contains("number 2 was inferred"));
    assert!(
        out.questions[3]
            .question
            .body
            .starts_with("Note: This question is part of a series")
    );
    assert_eq!(
        out.topics.iter().map(|t| t.number).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(plan_sets(&out, 50).len(), 2);
}

#[test]
fn guesses_exam_code_from_file_names() {
    use examcore::parse::guess_exam_code;
    assert_eq!(
        guess_exam_code("21042026 SC-100 OCR").as_deref(),
        Some("SC-100")
    );
    assert_eq!(guess_exam_code("AZ104 dump").as_deref(), Some("AZ104"));
    assert_eq!(guess_exam_code("21042026 export"), None);
}
