use crate::css::{Doc, is_custom_name_cont, parse_doc, print_doc};

#[derive(Debug, Clone)]
pub struct Page {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone)]
pub enum Segment {
    Text(String),
    StyleBody(Doc),
    ScriptBody(String),
}

impl Page {
    pub fn from_html(html: &str) -> Self {
        let mut segments: Vec<Segment> = Vec::new();
        let mut cursor = 0usize;
        while cursor < html.len() {
            let Some(hit) = find_next_block(html, cursor) else {
                push_text(&mut segments, &html[cursor..]);
                break;
            };
            // Emit the shell text up to and including the open tag.
            push_text(&mut segments, &html[cursor..hit.body_start]);
            let body = &html[hit.body_start..hit.body_end];
            match hit.kind {
                BlockKind::Style => segments.push(Segment::StyleBody(parse_doc(body))),
                BlockKind::Script => segments.push(Segment::ScriptBody(body.to_string())),
            }
            cursor = hit.body_end;
        }
        Page { segments }
    }

    pub fn print(&self) -> String {
        let cap: usize = self
            .segments
            .iter()
            .map(|s| match s {
                Segment::Text(t) => t.len(),
                Segment::ScriptBody(b) => b.len(),
                Segment::StyleBody(_) => 256,
            })
            .sum();
        let mut out = String::with_capacity(cap);
        for seg in &self.segments {
            match seg {
                Segment::Text(s) => out.push_str(s),
                Segment::StyleBody(d) => out.push_str(&print_doc(d)),
                Segment::ScriptBody(s) => out.push_str(s),
            }
        }
        out
    }

    pub fn style_docs(&self) -> impl Iterator<Item = &Doc> {
        self.segments.iter().filter_map(|s| match s {
            Segment::StyleBody(d) => Some(d),
            _ => None,
        })
    }

    pub fn style_docs_mut(&mut self) -> impl Iterator<Item = &mut Doc> {
        self.segments.iter_mut().filter_map(|s| match s {
            Segment::StyleBody(d) => Some(d),
            _ => None,
        })
    }

    pub fn scripts_mut(&mut self) -> impl Iterator<Item = &mut String> {
        self.segments.iter_mut().filter_map(|s| match s {
            Segment::ScriptBody(b) => Some(b),
            _ => None,
        })
    }

    pub fn text_and_scripts(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(|s| match s {
            Segment::Text(s) | Segment::ScriptBody(s) => Some(s.as_str()),
            _ => None,
        })
    }

    pub fn map_text_and_scripts(&mut self, mut transform: impl FnMut(&str) -> String) {
        for seg in &mut self.segments {
            match seg {
                Segment::Text(s) | Segment::ScriptBody(s) => *s = transform(s),
                Segment::StyleBody(_) => {}
            }
        }
    }
}

fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Segment::Text(last)) = segments.last_mut() {
        last.push_str(text);
        return;
    }
    segments.push(Segment::Text(text.to_string()));
}

#[derive(Copy, Clone)]
enum BlockKind {
    Style,
    Script,
}

struct BlockHit {
    kind: BlockKind,
    body_start: usize,
    body_end: usize,
}

fn find_next_block(html: &str, cursor: usize) -> Option<BlockHit> {
    let style = find_block(html, cursor, "<style", "</style>", BlockKind::Style);
    let script = find_block(html, cursor, "<script", "</script>", BlockKind::Script);
    [style, script]
        .into_iter()
        .flatten()
        .min_by_key(|h| h.body_start)
}

fn find_block(
    html: &str,
    cursor: usize,
    open_prefix: &str,
    close: &str,
    kind: BlockKind,
) -> Option<BlockHit> {
    let bytes = html.as_bytes();
    let mut search = cursor;
    while let Some(rel) = html[search..].find(open_prefix) {
        let open_at = search + rel;
        let after = open_at + open_prefix.len();
        let ok_boundary = bytes
            .get(after)
            .map(|&b| !is_custom_name_cont(b))
            .unwrap_or(false);
        if !ok_boundary {
            search = after;
            continue;
        }
        let gt_rel = html[after..].find('>')?;
        let body_start = after + gt_rel + 1;
        let close_rel = html[body_start..].find(close)?;
        let body_end = body_start + close_rel;
        return Some(BlockHit {
            kind,
            body_start,
            body_end,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn roundtrips(html: &str) {
        let page = Page::from_html(html);
        assert_eq!(page.print(), html, "roundtrip diverged");
    }

    #[test]
    fn roundtrips_empty() {
        roundtrips("");
    }

    #[test]
    fn roundtrips_plain_html() {
        roundtrips("<html><body>hi</body></html>");
    }

    #[test]
    fn roundtrips_one_style_block() {
        roundtrips("<style>.a { --x: 1; --y: calc(var(--x) + 1); }</style>");
    }

    #[test]
    fn roundtrips_style_with_attributes() {
        roundtrips("<style type=\"text/css\" id=\"main\">.a { --x: 1; }</style>");
    }

    #[test]
    fn canonicalises_css_decl_separator() {
        // The CSS pretty-printer always emits `--name: value;` with
        // exactly one space after the colon; whitespace elsewhere is
        // preserved verbatim.
        let page = Page::from_html("<style>.a{--x:1;}</style>");
        assert_eq!(page.print(), "<style>.a{--x: 1;}</style>");
    }

    #[test]
    fn roundtrips_multiple_styles_and_scripts() {
        let html = "<html>\n<head><style>.a { --x: 1; }</style></head>\n\
                    <body>\n<style>.b { --y: 2; }</style>\n\
                    <script>console.log('hi')</script>\n\
                    <script>var x = 1;</script>\n\
                    </body>\n</html>";
        roundtrips(html);
    }

    #[test]
    fn segments_partition_style_and_text() {
        let page = Page::from_html("pre<style>.a { --x: 1; }</style>post");
        assert_eq!(page.segments.len(), 3);
        match &page.segments[0] {
            Segment::Text(s) => assert_eq!(s, "pre<style>"),
            _ => panic!("expected text segment"),
        }
        match &page.segments[1] {
            Segment::StyleBody(_) => {}
            _ => panic!("expected style body"),
        }
        match &page.segments[2] {
            Segment::Text(s) => assert_eq!(s, "</style>post"),
            _ => panic!("expected text segment"),
        }
    }

    #[test]
    fn segments_partition_script() {
        let page = Page::from_html("<script>let a=1;</script>");
        assert_eq!(page.segments.len(), 3);
        match &page.segments[1] {
            Segment::ScriptBody(b) => assert_eq!(b, "let a=1;"),
            _ => panic!("expected script body"),
        }
    }

    #[test]
    fn does_not_mistake_styles_tag_for_style() {
        // The HTML uses `<styles>` (not a real tag, but here for
        // boundary testing). Must not be classified as `<style>`.
        let html = "<styles>not css</styles>";
        let page = Page::from_html(html);
        assert!(matches!(page.segments.as_slice(), [Segment::Text(_)]));
        assert_eq!(page.print(), html);
    }

    #[test]
    fn unterminated_style_passes_through_as_text() {
        let html = "<style>.a { --x: 1; ";
        let page = Page::from_html(html);
        assert_eq!(page.print(), html);
    }

    #[test]
    fn unterminated_open_tag_passes_through_as_text() {
        let html = "<style ";
        let page = Page::from_html(html);
        assert_eq!(page.print(), html);
    }

    #[test]
    fn parses_style_body_as_doc() {
        let page = Page::from_html("<style>.a { --x: 1; }</style>");
        let mut found = 0;
        for doc in page.style_docs() {
            for item in &doc.items {
                if let crate::css::Item::Rule(_) = item {
                    found += 1;
                }
            }
        }
        assert_eq!(found, 1);
    }

    #[test]
    fn mutating_style_doc_shows_up_in_print() {
        let mut page = Page::from_html("<style>.a { --x: 1; }</style>");
        for doc in page.style_docs_mut() {
            for item in &mut doc.items {
                if let crate::css::Item::Rule(r) = item
                    && let crate::css::Body::Decls(items) = &mut r.body
                {
                    for di in items {
                        if let crate::css::DeclItem::Decl(d) = di {
                            d.name = "--renamed".to_string();
                        }
                    }
                }
            }
        }
        let out = page.print();
        assert!(out.contains("--renamed: 1"), "got: {out}");
        assert!(!out.contains("--x:"));
    }

    #[test]
    fn map_text_and_scripts_touches_only_text_segments() {
        let mut page = Page::from_html("AAA<style>.a { --x: 1; }</style>BBB");
        page.map_text_and_scripts(|s| s.replace("AAA", "<<<"));
        let out = page.print();
        assert!(out.starts_with("<<<"));
        assert!(out.contains("--x: 1"));
    }

    #[test]
    fn map_text_and_scripts_rewrites_script_body() {
        let mut page = Page::from_html("<script>let a = 1;</script>");
        page.map_text_and_scripts(|s| s.replace("let", "var"));
        // Both the surrounding text and the script body should get
        // the transform — but only ScriptBody and Text segments are
        // touched. The replacement of "let" only matches inside the
        // script body here.
        assert_eq!(page.print(), "<script>var a = 1;</script>");
    }

    #[test]
    fn roundtrips_adjacent_blocks() {
        roundtrips("<style>.a { --x: 1; }</style><style>.b { --y: 2; }</style>");
    }

    #[test]
    fn roundtrips_realistic_emit_shape() {
        let html = "\
<!doctype html>
<html>
<head>
<style>
@property --pc { syntax: \"<integer>\"; initial-value: 0; inherits: true; }
.clk { --pc: 0; --r0: var(--_1r0); }
</style>
</head>
<body>
<div class=\"terminal\"></div>
<script>
const x = document.querySelector('.terminal');
console.log('--pc:', getComputedStyle(x).getPropertyValue('--pc'));
</script>
</body>
</html>
";
        roundtrips(html);
    }
}
