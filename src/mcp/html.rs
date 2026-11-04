//! A small, forgiving HTML parser for `write_html`.
//!
//! Agents write fragments of HTML with inline styles, not whole documents, so this handles
//! elements, attributes (quoted or not), void and self-closing tags, text with entities and
//! comments, and keeps `<svg>` subtrees as raw markup for the SVG importer. Mismatched
//! closing tags close up to the nearest matching open element, like browsers do.

#[derive(Debug, Clone, PartialEq)]
pub enum Html {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Lowercase tag name.
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Html>,
    /// The element's source markup (kept for `<svg>`).
    pub raw: Option<String>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Decode character references.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(';').map(|j| j + 1).filter(|j| *j <= 12);
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "middot" => Some('·'),
            "bull" => Some('•'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "times" => Some('×'),
            "rarr" => Some('→'),
            "larr" => Some('←'),
            "check" => Some('✓'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn eat_ws(&mut self) {
        let r = self.rest();
        self.pos += r.len() - r.trim_start().len();
    }

    fn name(&mut self) -> String {
        let r = self.rest();
        let n = r
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/' || c == '=')
            .unwrap_or(r.len());
        self.pos += n;
        r[..n].to_string()
    }

    /// Parse attributes up to `>`; returns (attrs, self_closing).
    fn attrs(&mut self) -> (Vec<(String, String)>, bool) {
        let mut out = Vec::new();
        loop {
            self.eat_ws();
            let r = self.rest();
            if r.is_empty() {
                return (out, false);
            }
            if r.starts_with("/>") {
                self.pos += 2;
                return (out, true);
            }
            if r.starts_with('>') {
                self.pos += 1;
                return (out, false);
            }
            if r.starts_with('/') {
                self.pos += 1;
                continue;
            }
            let k = self.name();
            if k.is_empty() {
                self.pos += r.chars().next().map_or(1, char::len_utf8);
                continue;
            }
            self.eat_ws();
            let mut v = String::new();
            if self.rest().starts_with('=') {
                self.pos += 1;
                self.eat_ws();
                let r = self.rest();
                if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let end = r[1..].find(q).map_or(r.len(), |e| e + 1);
                    v = r[1..end].to_string();
                    self.pos += (end + 1).min(r.len());
                } else {
                    let end = r
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(r.len());
                    v = r[..end].to_string();
                    self.pos += end;
                }
            }
            out.push((k.to_ascii_lowercase(), decode_entities(&v)));
        }
    }

    /// Byte offset of the matching `</tag>` for raw-text elements (counting nesting).
    fn skip_to_close(&mut self, tag: &str) -> usize {
        let lower = self.src.to_ascii_lowercase();
        let (open, close) = (format!("<{tag}"), format!("</{tag}"));
        let mut depth = 1;
        let mut i = self.pos;
        loop {
            let next_close = lower[i..].find(&close).map(|j| j + i);
            let next_open = lower[i..].find(&open).map(|j| j + i);
            match (next_open, next_close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    i = o + open.len();
                }
                (_, Some(c)) => {
                    depth -= 1;
                    if depth == 0 {
                        let end = lower[c..].find('>').map_or(lower.len(), |e| c + e + 1);
                        self.pos = end;
                        return c;
                    }
                    i = c + close.len();
                }
                _ => {
                    self.pos = self.src.len();
                    return self.src.len();
                }
            }
        }
    }
}

/// Parse an HTML fragment into a list of top-level nodes.
pub fn parse(src: &str) -> Vec<Html> {
    let mut p = Parser { src, pos: 0 };
    // Stack of open elements; index 0 is a synthetic root.
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut text = String::new();
    let flush = |text: &mut String, stack: &mut Vec<Element>| {
        if !text.is_empty() {
            let t = decode_entities(text);
            stack.last_mut().unwrap().children.push(Html::Text(t));
            text.clear();
        }
    };
    while p.pos < src.len() {
        let r = p.rest();
        if r.starts_with("<!--") {
            flush(&mut text, &mut stack);
            p.pos += r.find("-->").map_or(r.len(), |e| e + 3);
        } else if r.starts_with("<!") || r.starts_with("<?") {
            flush(&mut text, &mut stack);
            p.pos += r.find('>').map_or(r.len(), |e| e + 1);
        } else if r.starts_with("</") {
            flush(&mut text, &mut stack);
            p.pos += 2;
            let tag = p.name().to_ascii_lowercase();
            p.pos += p.rest().find('>').map_or(p.rest().len(), |e| e + 1);
            if let Some(i) = stack.iter().rposition(|e| e.tag == tag)
                && i > 0
            {
                while stack.len() > i {
                    let el = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(Html::Element(el));
                }
            }
        } else if r.starts_with('<') && r[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            flush(&mut text, &mut stack);
            let start = p.pos;
            p.pos += 1;
            let tag = p.name().to_ascii_lowercase();
            let (attrs, self_closing) = p.attrs();
            let mut el = Element {
                tag: tag.clone(),
                attrs,
                ..Default::default()
            };
            if tag == "svg" && !self_closing {
                p.skip_to_close("svg");
                el.raw = Some(src[start..p.pos].to_string());
                stack.last_mut().unwrap().children.push(Html::Element(el));
            } else if matches!(tag.as_str(), "script" | "style" | "textarea" | "title")
                && !self_closing
            {
                let body_start = p.pos;
                let end = p.skip_to_close(&tag);
                if tag == "textarea" {
                    el.children
                        .push(Html::Text(decode_entities(&src[body_start..end])));
                    stack.last_mut().unwrap().children.push(Html::Element(el));
                }
            } else if self_closing || VOID.contains(&tag.as_str()) {
                stack.last_mut().unwrap().children.push(Html::Element(el));
            } else {
                stack.push(el);
            }
        } else {
            let first = r.chars().next().map_or(1, char::len_utf8);
            let n = r[first..].find('<').map_or(r.len(), |e| e + first);
            text.push_str(&r[..n]);
            p.pos += n;
        }
    }
    flush(&mut text, &mut stack);
    while stack.len() > 1 {
        let el = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Html::Element(el));
    }
    stack.pop().unwrap().children
}

/// Inline (phrasing) elements: their content flows into the surrounding text.
pub fn is_inline(tag: &str) -> bool {
    matches!(
        tag,
        "span"
            | "b"
            | "strong"
            | "i"
            | "em"
            | "a"
            | "code"
            | "small"
            | "u"
            | "s"
            | "del"
            | "ins"
            | "sup"
            | "sub"
            | "label"
            | "br"
            | "mark"
            | "abbr"
            | "kbd"
            | "time"
            | "q"
            | "cite"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(h: &Html) -> &Element {
        match h {
            Html::Element(e) => e,
            _ => panic!("not an element: {h:?}"),
        }
    }

    #[test]
    fn parses_nested_elements_attrs_and_text() {
        let h = parse(
            r#"<div layer-name="Card" style="padding: 8px"><h1 class=title>Hi &amp; bye</h1><img src='a.png'/><br><p>x</div>"#,
        );
        assert_eq!(h.len(), 1);
        let d = el(&h[0]);
        assert_eq!(d.tag, "div");
        assert_eq!(d.attr("layer-name"), Some("Card"));
        assert_eq!(d.attr("style"), Some("padding: 8px"));
        assert_eq!(d.children.len(), 4);
        let h1 = el(&d.children[0]);
        assert_eq!(h1.attr("class"), Some("title"));
        assert_eq!(h1.children, vec![Html::Text("Hi & bye".into())]);
        assert_eq!(el(&d.children[1]).tag, "img");
        assert_eq!(el(&d.children[2]).tag, "br");
        // The unclosed <p> is closed by </div>.
        assert_eq!(el(&d.children[3]).children, vec![Html::Text("x".into())]);
    }

    #[test]
    fn keeps_svg_raw_and_skips_comments_and_scripts() {
        let h = parse(
            r#"<!-- c --><svg width="10" height="10"><svg><rect/></svg></svg><script>if (a<b) {}</script><p>&#x41;&#66;</p>•<b>é</b>"#,
        );
        assert_eq!(h.len(), 4);
        assert_eq!(h[2], Html::Text("•".into()));
        let s = el(&h[0]);
        assert_eq!(s.tag, "svg");
        assert!(s.raw.as_ref().unwrap().ends_with("</svg></svg>"));
        assert_eq!(el(&h[1]).children, vec![Html::Text("AB".into())]);
    }
}
