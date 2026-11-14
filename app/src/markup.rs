//! A small Markdown subset to Pango markup, for model answers: code blocks,
//! inline code, bold, italics, headings and bullet lists. Everything is
//! escaped first, so model output can never inject markup.

use std::sync::LazyLock;

use regex::Regex;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn to_pango(text: &str) -> String {
    static CODE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```[^\n]*\n(.*?)(```|$)").unwrap());
    static INLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());
    static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(\S(?:[^\n]*?\S)?)\*\*").unwrap());
    static ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|[^\w*])\*(\S(?:[^*\n]*?\S)?)\*([^\w*]|$)").unwrap());
    static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+(.*)$").unwrap());
    static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(\s*)[-*]\s+").unwrap());
    static STASHED: LazyLock<Regex> = LazyLock::new(|| Regex::new("\x00(\\d+)\x00").unwrap());
    let mut blocks: Vec<String> = vec![];
    let text = CODE_BLOCK.replace_all(text, |c: &regex::Captures| {
        blocks.push(format!("<tt>{}</tt>", escape(c[1].trim_end())));
        format!("\x00{}\x00", blocks.len() - 1)
    });
    let text = escape(&text);
    let text = INLINE.replace_all(&text, "<tt>$1</tt>");
    let text = BOLD.replace_all(&text, "<b>$1</b>");
    let text = ITALIC.replace_all(&text, "$1<i>$2</i>$3");
    let text = HEADING.replace_all(&text, "<b>$1</b>");
    let text = BULLET.replace_all(&text, "$1• ");
    STASHED.replace_all(&text, |c: &regex::Captures| blocks[c[1].parse::<usize>().unwrap()].clone()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::to_pango;

    #[test]
    fn markdown_subset() {
        assert_eq!(to_pango("**bold** and *it* and `x<y`"), "<b>bold</b> and <i>it</i> and <tt>x&lt;y</tt>");
        assert_eq!(to_pango("# Title\n- one\n* two"), "<b>Title</b>\n• one\n• two");
        assert_eq!(to_pango("```sh\necho **hi** <x>\n```"), "<tt>echo **hi** &lt;x&gt;</tt>");
        assert_eq!(to_pango("a * b * c"), "a * b * c");
        assert_eq!(to_pango("<script>"), "&lt;script&gt;");
    }
}
