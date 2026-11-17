//! Questions about long documents, in their own clean context (not poured
//! into the chat): the document is cut into parts, the parts that best match
//! the question are picked, and the model answers from those alone. A
//! summary reads a few parts spread over the document, then combines them.

use std::path::Path;

use crate::tool::{Args, Context, ToolResult, arg_str};

use super::files::{home_path, short};

/// About 1.2K tokens a part, three parts an answer: room in an 8K context.
const PART_CHARS: usize = 4000;
const PARTS_PER_ANSWER: usize = 3;
const SUMMARY_PARTS: usize = 4;
const PAGES: u32 = 200;

const ANSWER: &str = "Answer the question only from the document excerpts below, briefly, in the language of the \
question. Copy numbers, dates and names exactly. If the excerpts don't say, answer exactly: The document doesn't say.";
const SUMMARIZE: &str = "Summarize this part of a document in 3 short bullet points: what it is, the key facts, \
numbers and dates. Plain text.";
const COMBINE: &str = "These are summaries of the parts of one document, in order. Write one short summary of the \
whole document (what it is, the key facts, numbers and dates) in the language of the request.";

/// Parts of about `size` characters, cut at paragraph or line ends.
pub fn parts(text: &str, size: usize) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if !cur.is_empty() && cur.len() + para.len() > size {
            out.push(std::mem::take(&mut cur));
        }
        if para.len() > size {
            // One huge paragraph (or a PDF with no blank lines): cut at lines, then hard.
            for line in para.lines() {
                if !cur.is_empty() && cur.len() + line.len() > size {
                    out.push(std::mem::take(&mut cur));
                }
                let mut l = line;
                while l.len() > size {
                    let cut = l
                        .char_indices()
                        .take_while(|(i, _)| *i < size)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(l.len());
                    out.push(l[..cut].to_string());
                    l = &l[cut..];
                }
                cur.push_str(l);
                cur.push('\n');
            }
        } else {
            cur.push_str(para);
            cur.push_str("\n\n");
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Word stems for matching (4 letters: "kaucja"/"kaucji", "deposit"/"deposits").
fn stems(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.chars().take(5).collect())
        .collect()
}

/// The indexes of the parts that best match the question, in document order.
pub fn best_parts(parts: &[String], question: &str, n: usize) -> Vec<usize> {
    let q: Vec<String> = stems(question);
    let mut scored: Vec<(usize, f64)> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let words = stems(p);
            let hits: f64 = q
                .iter()
                .map(|w| {
                    let c = words
                        .iter()
                        .filter(|x| x.starts_with(w.as_str()) || w.starts_with(x.as_str()))
                        .count();
                    if c == 0 { 0.0 } else { 1.0 + (c as f64).ln() } // each query word counts, repeats a little
                })
                .sum();
            let digits = if question.chars().any(|c| c.is_ascii_digit())
                || q.iter()
                    .any(|w| ["how", "ile", "kiedy", "when"].contains(&w.as_str()))
            {
                p.chars().filter(|c| c.is_ascii_digit()).count().min(40) as f64 / 40.0
            } else {
                0.0
            };
            (i, hits + digits)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut pick: Vec<usize> = scored.into_iter().take(n).map(|(i, _)| i).collect();
    pick.sort();
    pick
}

/// Evenly spread parts for a summary.
fn spread(count: usize, n: usize) -> Vec<usize> {
    if count <= n {
        return (0..count).collect();
    }
    (0..n).map(|k| k * (count - 1) / (n - 1)).collect()
}

fn wants_summary(question: &str) -> bool {
    let q = question.to_lowercase();
    [
        "summar",
        "sum up",
        "overview",
        "what is it about",
        "what's it about",
        "tl;dr",
        "tldr",
        "streszcz",
        "podsumuj",
        "o czym",
        "co to jest",
    ]
    .iter()
    .any(|w| q.contains(w))
}

pub fn ask_document(ctx: &Context, args: &Args) -> ToolResult {
    let path = home_path(&arg_str(args, "path"), true)?;
    let question = arg_str(args, "question");
    let question = if question.trim().is_empty() {
        "Summarize it.".to_string()
    } else {
        question
    };
    let text = super::localsearch::document_text(&path, PAGES)?;
    let all = parts(&text, PART_CHARS);
    let name = short(Path::new(&path));
    let label = |i: usize| format!("[part {} of {}]", i + 1, all.len());
    let Some(ask) = &ctx.ask_model else {
        // No model to hand (tests): give the best passages back to the chat.
        let pick = best_parts(&all, &question, PARTS_PER_ANSWER);
        return Ok(format!(
            "{name}:\n{}",
            pick.iter()
                .map(|i| format!("{}\n{}", label(*i), all[*i]))
                .collect::<Vec<_>>()
                .join("\n\n")
        ));
    };
    if wants_summary(&question) {
        let notes: Vec<String> = spread(all.len(), SUMMARY_PARTS)
            .into_iter()
            .map(|i| ask(SUMMARIZE, &all[i]).map(|s| format!("{} {s}", label(i))))
            .collect::<Result<_, _>>()?;
        let whole = ask(
            COMBINE,
            &format!("Request: {question}\n\n{}", notes.join("\n\n")),
        )?;
        let covered = if all.len() > SUMMARY_PARTS {
            format!(" (from {SUMMARY_PARTS} of its {} parts)", all.len())
        } else {
            String::new()
        };
        return Ok(format!("Summary of {name}{covered}:\n{whole}"));
    }
    let pick = best_parts(&all, &question, PARTS_PER_ANSWER);
    let excerpts = pick
        .iter()
        .map(|i| format!("{}\n{}", label(*i), all[*i]))
        .collect::<Vec<_>>()
        .join("\n\n");
    let answer = ask(
        ANSWER,
        &format!("Question: {question}\n\nExcerpts from {name}:\n{excerpts}"),
    )?;
    Ok(format!("From {name}: {}", answer.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_and_picking() {
        let doc = format!(
            "{}\n\n§4 Deposit: the Tenant pays a deposit (kaucja) of 4 500 zł.\n\n{}",
            "Intro text about the flat. ".repeat(200),
            "Rules about pets and noise. ".repeat(200)
        );
        let p = parts(&doc, 1000);
        assert!(
            p.len() > 5 && p.iter().all(|x| x.len() <= 1100),
            "{:?}",
            p.iter().map(|x| x.len()).collect::<Vec<_>>()
        );
        let best = best_parts(&p, "how big is the deposit?", 1);
        assert!(p[best[0]].contains("4 500"), "{}", p[best[0]]);
        let pl = best_parts(&p, "ile wynosi kaucja?", 1);
        assert!(p[pl[0]].contains("kaucja"));
        assert_eq!(spread(10, 4), [0, 3, 6, 9]);
        assert!(wants_summary("podsumuj ten dokument") && !wants_summary("what's the deposit?"));
        assert!(
            parts(&"x".repeat(9000), 4000).len() == 3,
            "a run with no breaks is cut too"
        );
    }

    #[test]
    fn answers_through_the_model_hook() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lease.txt");
        std::fs::write(
            &file,
            format!(
                "{}\n\nThe deposit is 4 500 zł.",
                "Filler paragraph about nothing. ".repeat(300)
            ),
        )
        .unwrap();
        let _h = crate::store::testenv::temp_home();
        let home = super::super::files::real_home();
        std::fs::copy(&file, home.join("lease.txt")).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let s = seen.clone();
        let ctx = Context {
            ask_model: Some(std::sync::Arc::new(move |_sys: &str, user: &str| {
                *s.lock().unwrap() = user.to_string();
                Ok("4 500 zł.".into())
            })),
            ..Default::default()
        };
        let args: Args = serde_json::from_value(
            serde_json::json!({"path": "lease.txt", "question": "how big is the deposit?"}),
        )
        .unwrap();
        assert_eq!(
            ask_document(&ctx, &args).unwrap(),
            "From ~/lease.txt: 4 500 zł."
        );
        assert!(
            seen.lock().unwrap().contains("The deposit is 4 500"),
            "the matching part went to the model"
        );
    }
}
