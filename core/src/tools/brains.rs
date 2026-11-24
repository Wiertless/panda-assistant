//! Memory and skills as tools.

use crate::store;
use crate::tool::{Args, Context, ToolError, ToolResult, arg_str};

pub fn remember(ctx: &Context, args: &Args) -> ToolResult {
    let fact = arg_str(args, "fact");
    let out = store::remember(&fact).map_err(ToolError)?;
    if out.starts_with("Remembered") {
        ctx.can_undo(&format!("Remember “{}”", fact.trim()), move || {
            store::forget(&fact);
            Ok("forgotten again.".into())
        });
    }
    Ok(out)
}

/// "Undo that": reverse the newest change (a setting, the volume, a reminder, a remembered fact…).
pub fn undo(ctx: &Context, args: &Args) -> ToolResult {
    match args.get("whole_task") {
        Some(serde_json::Value::Bool(true)) => ctx.undo_task(),
        Some(serde_json::Value::String(s)) if s == "true" => ctx.undo_task(),
        _ => ctx.undo_last(),
    }
}

pub fn forget(_: &Context, args: &Args) -> ToolResult {
    let gone = store::forget(&arg_str(args, "fact"));
    if gone.is_empty() {
        return Err("nothing remembered matches that".into());
    }
    Ok(format!("Forgot: {}", gone.join("; ")))
}

pub fn use_skill(_: &Context, args: &Args) -> ToolResult {
    let name = arg_str(args, "name");
    match store::find_skill(&name) {
        Some(s) => Ok(format!(
            "Skill “{}”. Do these steps now, using tools:\n{}",
            s.title, s.steps
        )),
        None => {
            let names: Vec<String> = store::skills().into_iter().map(|s| s.title).collect();
            let list = if names.is_empty() {
                "none saved".to_string()
            } else {
                names.join(", ")
            };
            Err(format!("no skill called '{name}' (skills: {list})").into())
        }
    }
}

pub fn save_skill(_: &Context, args: &Args) -> ToolResult {
    let s = store::save_skill(
        &arg_str(args, "name"),
        &arg_str(args, "steps"),
        &arg_str(args, "description"),
    )
    .map_err(ToolError)?;
    Ok(format!(
        "Saved the skill “{}”. Run it any time with /{}.",
        s.title, s.slug
    ))
}
