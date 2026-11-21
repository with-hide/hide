//! An HTML file a phone draws itself (issue with-hide/hide-private#689):
//! the page and the files beside it that it names, folded into one document.
//!
//! The phone cannot ask for a file by name (D-16), so everything a page needs
//! from its checkout has to arrive with it. This file reads which files a page
//! (and each stylesheet it loads) names by a relative address, and writes each
//! one back into the text as a `data:` address, so the phone gets one
//! self-contained document. What it names on the internet stays as written;
//! the phone's frame fetches that itself. The desktop's rule for a local page
//! holds here too: a file outside the page's checkout is never read.
//!
//! The reading is a scan, not an HTML parser: it finds `src`, `href`,
//! `poster` and `srcset` values and CSS `url(...)` and `@import` strings.
//! Taking a few strings that are not addresses is harmless, since only a name
//! that resolves to a file of a known resource type inside the checkout is
//! read; an address the scan misses stays as written and draws as missing.

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hide_platform::path::{RelPath, wire_join, wire_relative};
use percent_encoding::percent_decode_str;

/// Files one page may fold in, stylesheets' own files included.
pub const MAX_FILES: usize = 64;
/// How deep the files a stylesheet names are followed: the page, its
/// stylesheets, and what those stylesheets name.
pub const MAX_DEPTH: usize = 2;

/// Where one page sits: its checkout's root and its own path below it, both
/// in wire spelling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Place {
    pub checkout: String,
    pub page: RelPath,
}

impl Place {
    /// The page at the wire path `path` of the checkout `checkout`; `None`
    /// when the path is not below the checkout.
    pub fn of(checkout: &str, path: &str) -> Option<Self> {
        let page = wire_relative(checkout, path).ok()?;
        (!page.is_root()).then(|| Self {
            checkout: checkout.to_owned(),
            page,
        })
    }

    /// The wire path of a file of this checkout.
    pub fn wire(&self, file: &RelPath) -> String {
        wire_join(&self.checkout, file)
    }
}

/// The resource types a page folds in, by extension: what a browser loads
/// for a page to draw. A link to another page (`.html`) is not one.
fn mime(file: &RelPath) -> Option<&'static str> {
    let name = file.file_name()?;
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(match extension.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        _ => return None,
    })
}

/// Whether a folded-in file is a stylesheet, whose own addresses are folded
/// in as the page's are.
pub fn is_stylesheet(file: &RelPath) -> bool {
    mime(file) == Some("text/css")
}

/// One address in a text: where its value is, and the value.
#[derive(Debug, Eq, PartialEq)]
struct Address {
    start: usize,
    end: usize,
}

/// The file of `checkout` an address names from the folder `dir`, when it
/// names one: a relative address, without its query or fragment, decoded,
/// that stays inside the checkout and has a resource type.
fn resolve(dir: &RelPath, address: &str) -> Option<RelPath> {
    let address = address.trim();
    let first = address.chars().next()?;
    if matches!(first, '#' | '/' | '\\' | '?') || has_scheme(address) {
        return None;
    }
    let path = &address[..address.find(['?', '#']).unwrap_or(address.len())];
    let decoded = percent_decode_str(path).decode_utf8().ok()?;
    let mut names: Vec<String> = dir.names().map(str::to_owned).collect();
    for name in decoded.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                // Above the checkout's root is outside it.
                names.pop()?;
            }
            name => names.push(name.to_owned()),
        }
    }
    let file = names
        .iter()
        .try_fold(RelPath::root(), |path, name| path.join(name).ok())?;
    mime(&file).map(|_| file)
}

/// `data:`, `http:`, `mailto:`, `javascript:` and any other scheme.
fn has_scheme(address: &str) -> bool {
    let Some((scheme, _)) = address.split_once(':') else {
        return false;
    };
    let mut letters = scheme.chars();
    letters.next().is_some_and(|c| c.is_ascii_alphabetic())
        && letters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The addresses a text names: HTML attribute values (`src`, `href`,
/// `poster`, each candidate of `srcset`) unless `css`, and CSS `url(...)`
/// and `@import` strings in both.
fn addresses(text: &str, css: bool) -> Vec<Address> {
    let mut found = Vec::new();
    if !css {
        for name in ["src", "href", "poster"] {
            for (start, end) in attribute_values(text, name) {
                found.push(Address { start, end });
            }
        }
        for (start, end) in attribute_values(text, "srcset") {
            found.extend(srcset(text, start, end));
        }
    }
    found.extend(css_urls(text));
    found.extend(css_imports(text));
    found.sort_by_key(|address| address.start);
    // A `src` scan also matches inside `srcset`'s own name, never its value,
    // but two scans may still meet one span: keep the first of an overlap.
    let mut kept: Vec<Address> = Vec::with_capacity(found.len());
    for address in found {
        if kept.last().is_none_or(|last| address.start >= last.end) {
            kept.push(address);
        }
    }
    kept
}

/// The value spans of every `name=` attribute: the name must stand alone
/// (after a space, a quote or a tag's `<`), then optional spaces, `=`,
/// optional spaces, and a quoted or bare value.
fn attribute_values(text: &str, name: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before = at.checked_sub(1).map(|i| bytes[i]);
        if !before.is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'/')) {
            continue;
        }
        let mut i = at + name.len();
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&open) = bytes.get(i) else { break };
        let (start, end) = if matches!(open, b'"' | b'\'') {
            let start = i + 1;
            let Some(len) = bytes[start..].iter().position(|&b| b == open) else {
                continue;
            };
            (start, start + len)
        } else {
            let len = bytes[i..]
                .iter()
                .position(|&b| b.is_ascii_whitespace() || b == b'>')
                .unwrap_or(bytes.len() - i);
            (i, i + len)
        };
        if end > start {
            spans.push((start, end));
            from = end;
        }
    }
    spans
}

/// Each candidate address of a `srcset` value: comma-separated, an address
/// then an optional descriptor.
fn srcset(text: &str, start: usize, end: usize) -> Vec<Address> {
    let mut found = Vec::new();
    let mut at = start;
    for candidate in text[start..end].split(',') {
        let lead = candidate.len() - candidate.trim_start().len();
        let address_len = candidate
            .trim_start()
            .find(char::is_whitespace)
            .unwrap_or(candidate.trim_start().len());
        if address_len > 0 {
            found.push(Address {
                start: at + lead,
                end: at + lead + address_len,
            });
        }
        at += candidate.len() + 1;
    }
    found
}

/// The address inside each `url(...)`, quoted or not.
fn css_urls(text: &str) -> Vec<Address> {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("url(") {
        let mut i = from + at + 4;
        from = i;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let quote = bytes.get(i).copied().filter(|b| matches!(b, b'"' | b'\''));
        let start = i + usize::from(quote.is_some());
        let close = |b: &u8| match quote {
            Some(q) => *b == q,
            None => *b == b')' || b.is_ascii_whitespace(),
        };
        let Some(len) = bytes[start..].iter().position(close) else {
            continue;
        };
        if len > 0 {
            found.push(Address {
                start,
                end: start + len,
            });
        }
        from = start + len;
    }
    found
}

/// The quoted string of each `@import "..."` (the `url(...)` form is
/// `css_urls`').
fn css_imports(text: &str) -> Vec<Address> {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("@import") {
        let mut i = from + at + 7;
        from = i;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&quote) = bytes.get(i).filter(|b| matches!(b, b'"' | b'\'')) else {
            continue;
        };
        let start = i + 1;
        if let Some(len) = bytes[start..].iter().position(|&b| b == quote) {
            found.push(Address {
                start,
                end: start + len,
            });
            from = start + len;
        }
    }
    found
}

/// The files of the checkout a text names from the folder `dir`, each once,
/// in the order they first appear.
pub fn wanted(text: &str, dir: &RelPath, css: bool) -> Vec<RelPath> {
    let mut files: Vec<RelPath> = Vec::new();
    for address in addresses(text, css) {
        if let Some(file) = resolve(dir, &text[address.start..address.end])
            && !files.contains(&file)
        {
            files.push(file);
        }
    }
    files
}

/// `text` with every address that names a file of `folded` replaced by that
/// file's `data:` address; any other address stays as written.
pub fn fold(text: &str, dir: &RelPath, css: bool, folded: &HashMap<RelPath, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for address in addresses(text, css) {
        let Some(data) =
            resolve(dir, &text[address.start..address.end]).and_then(|file| folded.get(&file))
        else {
            continue;
        };
        out.push_str(&text[at..address.start]);
        out.push_str(data);
        at = address.end;
    }
    out.push_str(&text[at..]);
    out
}

/// A file's `data:` address.
fn data_address(file: &RelPath, bytes: &[u8]) -> Option<String> {
    Some(format!(
        "data:{};base64,{}",
        mime(file)?,
        STANDARD.encode(bytes)
    ))
}

/// The `data:` address of each file read, in the order the page reached them
/// (stylesheets' files after the stylesheets): every other file first, then
/// the stylesheets last reached first, so a stylesheet has what it names
/// folded in before it is folded itself.
pub fn data_addresses(files: &[(RelPath, Vec<u8>)]) -> HashMap<RelPath, String> {
    let mut folded = HashMap::new();
    let (sheets, others): (Vec<_>, Vec<_>) =
        files.iter().partition(|(file, _)| is_stylesheet(file));
    for (file, bytes) in others.into_iter().chain(sheets.into_iter().rev()) {
        let data = if is_stylesheet(file) {
            let dir = file.parent().unwrap_or_default();
            let css = String::from_utf8_lossy(bytes);
            data_address(file, fold(&css, &dir, true, &folded).as_bytes())
        } else {
            data_address(file, bytes)
        };
        if let Some(data) = data {
            folded.insert(file.clone(), data);
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(path: &str) -> RelPath {
        RelPath::parse(path).unwrap()
    }

    fn names(files: &[RelPath]) -> Vec<&str> {
        files.iter().map(RelPath::as_str).collect()
    }

    #[test]
    fn a_page_names_the_files_beside_it_and_not_the_web_or_other_pages() {
        let page = r##"<!doctype html>
<link rel="stylesheet" href="style.css">
<link rel=icon href=img/icon.png>
<script src='app.js'></script>
<script src="https://cdn.tailwindcss.com"></script>
<img src="img/hero.png?v=2#top" alt="">
<img srcset="img/a.png 1x, img/a-2x.png 2x">
<video poster="../shared/poster.jpg"></video>
<a href="other.html">next</a>
<a href="#top">top</a>
<img src="data:image/png;base64,AAAA">
<div style="background: url('img/bg.webp')"></div>
<img src="img/hero.png">"##;
        assert_eq!(
            names(&wanted(page, &rel("design"), false)),
            [
                "design/style.css",
                "design/img/icon.png",
                "design/app.js",
                "design/img/hero.png",
                "design/img/a.png",
                "design/img/a-2x.png",
                "shared/poster.jpg",
                "design/img/bg.webp",
            ]
        );
    }

    #[test]
    fn an_address_outside_the_checkout_or_absolute_names_nothing() {
        let page = r#"<img src="../../etc/passwd.png"><img src="/Users/me/a.png"><img src="\\server\a.png"><img src="file:///a.png">"#;
        assert!(wanted(page, &rel("design"), false).is_empty());
        assert_eq!(
            names(&wanted(r#"<img src="../a.png">"#, &rel("design"), false)),
            ["a.png"]
        );
        assert!(wanted(r#"<img src="../a.png">"#, &RelPath::root(), false).is_empty());
    }

    #[test]
    fn an_encoded_name_is_decoded_to_its_file() {
        assert_eq!(
            names(&wanted(
                r#"<img src="my%20shot.png">"#,
                &RelPath::root(),
                false
            )),
            ["my shot.png"]
        );
    }

    #[test]
    fn a_stylesheet_names_its_fonts_and_imports_but_no_attribute() {
        let css = r#"@import "base.css";
@font-face { src: url(fonts/inter.woff2) format("woff2"); }
.hero { background: url( "../img/bg.png" ); }
.x { content: "src=nope.png"; }"#;
        assert_eq!(
            names(&wanted(css, &rel("css"), true)),
            ["css/base.css", "css/fonts/inter.woff2", "img/bg.png"]
        );
    }

    #[test]
    fn folding_replaces_each_address_of_a_folded_file_and_keeps_the_rest() {
        let page = r#"<img src="a.png"><img src="b.png"><img src="https://x.dev/c.png"><img src = 'a.png'>"#;
        let folded = HashMap::from([(rel("a.png"), "data:image/png;base64,QQ==".to_owned())]);
        assert_eq!(
            fold(page, &RelPath::root(), false, &folded),
            r#"<img src="data:image/png;base64,QQ=="><img src="b.png"><img src="https://x.dev/c.png"><img src = 'data:image/png;base64,QQ=='>"#
        );
    }

    #[test]
    fn a_stylesheet_carries_the_files_it_names_and_an_import_its_own() {
        let files = vec![
            (
                rel("css/site.css"),
                br#"@import "base.css"; .a { background: url(../img/a.png) }"#.to_vec(),
            ),
            (rel("img/a.png"), b"PNG".to_vec()),
            (
                rel("css/base.css"),
                b".b { background: url('../img/a.png') }".to_vec(),
            ),
        ];
        let folded = data_addresses(&files);
        let decode = |file: &str| {
            let data = &folded[&rel(file)];
            let encoded = data.split_once(";base64,").unwrap().1;
            String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap()
        };
        assert_eq!(folded[&rel("img/a.png")], "data:image/png;base64,UE5H");
        assert_eq!(
            decode("css/base.css"),
            ".b { background: url('data:image/png;base64,UE5H') }"
        );
        let site = decode("css/site.css");
        assert!(
            site.starts_with(r#"@import "data:text/css;base64,"#),
            "{site}"
        );
        assert!(
            site.ends_with(".a { background: url(data:image/png;base64,UE5H) }"),
            "{site}"
        );
    }

    #[test]
    fn a_data_address_carries_the_type_by_extension() {
        assert_eq!(
            data_address(&rel("a.css"), b"a{}").as_deref(),
            Some("data:text/css;base64,YXt9")
        );
        assert_eq!(data_address(&rel("a.html"), b"x"), None);
        assert!(is_stylesheet(&rel("x/A.CSS")));
    }

    #[test]
    fn a_place_is_a_page_below_its_checkout() {
        let place = Place::of("/repo", "/repo/design/mock.html").unwrap();
        assert_eq!(place.page.as_str(), "design/mock.html");
        assert_eq!(place.wire(&rel("design/a.png")), "/repo/design/a.png");
        assert_eq!(Place::of("/repo", "/repo-other/mock.html"), None);
        assert_eq!(
            Place::of("C:/repo", "C:/repo/m.html")
                .unwrap()
                .wire(&rel("a.png")),
            "C:/repo/a.png"
        );
    }
}
