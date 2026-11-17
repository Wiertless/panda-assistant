//! One search across the user's things: file names, what's inside files, contacts,
//! calendar and mail, at once (each is its own tool too). For questions where the
//! model can't tell where the answer lives ("when's my flight?").

use super::{files, localsearch, mail, personal};
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

type Search = fn(&Context, &Args) -> ToolResult;

const SECTION_CHARS: usize = 1500;

pub fn search_everything(ctx: &Context, args: &Args) -> ToolResult {
    let q = arg_str(args, "query").trim().to_string();
    if q.is_empty() {
        return Err("say what to look for".into());
    }
    let arg = |k: &str| -> Args {
        [(k.to_string(), serde_json::Value::String(q.clone()))]
            .into_iter()
            .collect()
    };
    let searches: [(&str, Search, Args); 5] = [
        ("Calendar", personal::find_events, arg("query")),
        ("Mail", mail::search_mail, arg("query")),
        ("Files named like it", files::find_files, arg("name")),
        (
            "Inside files",
            localsearch::search_file_contents,
            arg("query"),
        ),
        ("Contacts", personal::find_contact, arg("name")),
    ];
    let found: Vec<String> = std::thread::scope(|s| {
        let jobs: Vec<_> = searches
            .iter()
            .map(|(label, run, a)| (label, s.spawn(move || run(ctx, a))))
            .collect();
        jobs.into_iter()
            .filter_map(|(label, j)| match j.join() {
                Ok(Ok(r)) if !r.starts_with("No ") => {
                    let r: String = r.chars().take(SECTION_CHARS).collect();
                    Some(format!("## {label}\n{r}"))
                }
                _ => None,
            })
            .collect()
    });
    if found.is_empty() {
        return Err(ToolError::cannot(format!(
            "nothing about “{q}” in your calendar, mail, files or contacts"
        )));
    }
    Ok(found.join("\n\n"))
}
