//! The format-neutral half of a thread read: each format's converter hands
//! the builder what one record says (a message part, a tool call, a result,
//! a plan) in plain values, and the builder alone shapes ACP updates, applies
//! the caps and marks what Hide or the harness wrapped around the operator's
//! words. A converter never builds an ACP value itself, so every format draws
//! the same thread for the same kind of record.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use agent_client_protocol_schema::v1::{
    Content, ContentBlock, ContentChunk, Diff, ImageContent, Meta, Plan, PlanEntry,
    PlanEntryPriority, PlanEntryStatus, ResourceLink, SessionUpdate, TextContent, ToolCall,
    ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind,
};
use serde_json::{Value, json};

use super::{IMAGE_LIMIT_BYTES, TEXT_LIMIT_CHARS};

/// One piece of a message as a record holds it.
#[derive(Debug)]
pub(crate) enum Part<'a> {
    Text(&'a str),
    /// Base64 image data the record carries.
    Image {
        data: &'a str,
        mime: &'a str,
    },
    /// An image the record names by its path only (D-09), with its type
    /// when the record states it.
    ImagePath {
        path: String,
        mime: Option<&'a str>,
    },
}

/// What a person's message is, beyond the operator's own words. Converters
/// pass what native metadata proves; the builder adds what the text shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Origin {
    #[default]
    Operator,
    /// Context the harness, a hook or Hide put in the conversation.
    Injected,
    /// A Hide letter delivered as a prompt.
    Letter,
    /// The record of an interrupted turn.
    Interrupted,
}

impl Origin {
    const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Operator => None,
            Self::Injected => Some("injected"),
            Self::Letter => Some("letter"),
            Self::Interrupted => Some("interrupted"),
        }
    }
}

/// A tool call as its record states it.
#[derive(Debug)]
pub(crate) struct Call {
    pub id: String,
    /// The tool's own name, as the agent calls it.
    pub name: String,
    pub title: String,
    pub kind: ToolKind,
    pub input: Option<Value>,
    /// Native paths the call works on, in the order the call names them.
    pub paths: Vec<String>,
    /// A file change the call's input already states in full.
    pub diffs: Vec<FileDiff>,
    /// Calls whose status the record states at once, such as a call that
    /// carries its own result.
    pub status: ToolCallStatus,
}

impl Call {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: ToolKind) -> Self {
        let name = name.into();
        Self {
            id: id.into(),
            title: name.clone(),
            name,
            kind,
            input: None,
            paths: Vec::new(),
            diffs: Vec::new(),
            status: ToolCallStatus::Pending,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn input(mut self, input: &Value) -> Self {
        self.input = Some(input.clone());
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }

    pub fn diff(mut self, diff: FileDiff) -> Self {
        self.diffs.push(diff);
        self
    }
}

/// A file before and after a change; `before` is `None` for a new file.
#[derive(Debug)]
pub(crate) struct FileDiff {
    pub path: String,
    pub before: Option<String>,
    pub after: String,
}

/// What a call's result record says.
#[derive(Debug, Default)]
pub(crate) struct Outcome<'a> {
    pub failed: bool,
    /// The call is still running: an interim report, not its end.
    pub running: bool,
    pub output: Option<String>,
    pub images: Vec<Part<'a>>,
    pub diffs: Vec<FileDiff>,
}

/// One plan step.
#[derive(Debug)]
pub(crate) struct Step {
    pub text: String,
    pub status: PlanEntryStatus,
}

/// The updates of one page, in record order, and what the page passed over.
#[derive(Debug, Default)]
pub(crate) struct Thread {
    updates: Vec<SessionUpdate>,
    skipped: BTreeMap<&'static str, usize>,
    /// Calls waiting for a result that names no call, oldest first: a format
    /// whose results follow their calls in order pairs them here (D-10).
    awaiting: VecDeque<String>,
    /// The calls this page has drawn, so a call a record states again
    /// refines the one drawn rather than drawing a second.
    called: BTreeSet<String>,
    /// Whether a record that should hold conversation could not be read.
    unreadable: bool,
}

impl Thread {
    pub fn into_parts(self) -> (Vec<SessionUpdate>, BTreeMap<&'static str, usize>) {
        (self.updates, self.skipped)
    }

    pub fn skip(&mut self, reason: &'static str) {
        *self.skipped.entry(reason).or_default() += 1;
    }

    /// A record that should hold conversation but whose shape the
    /// converter does not know: counted, and if nothing on the page could be
    /// read the page is refused rather than answered empty (B9).
    pub fn unreadable(&mut self, reason: &'static str) {
        self.unreadable = true;
        self.skip(reason);
    }

    /// Whether every conversation record of the page failed.
    pub fn nothing_readable(&self) -> bool {
        self.unreadable && self.updates.is_empty()
    }

    /// A person's message. Text wrapped as pasted content becomes its own
    /// block, so the operator's typed words stay apart from what they pasted.
    pub fn user<'a>(
        &mut self,
        message: Option<&str>,
        origin: Origin,
        parts: impl IntoIterator<Item = Part<'a>>,
    ) {
        for part in parts {
            match part {
                Part::Text(text) => {
                    let origin = classify(origin, text);
                    for (piece, pasted) in pasted_pieces(text) {
                        let block = text_block(piece, pasted.then_some(("pasted", json!(true))));
                        self.chunk(SessionUpdate::UserMessageChunk, message, origin, block);
                    }
                }
                part => {
                    if let Some(block) = media_block(part) {
                        self.chunk(SessionUpdate::UserMessageChunk, message, origin, block);
                    }
                }
            }
        }
    }

    pub fn agent<'a>(&mut self, message: Option<&str>, part: Part<'a>) {
        let block = match part {
            Part::Text(text) if text.trim().is_empty() => return,
            Part::Text(text) => Some(text_block(text, None)),
            part => media_block(part),
        };
        if let Some(block) = block {
            self.chunk(
                SessionUpdate::AgentMessageChunk,
                message,
                Origin::Operator,
                block,
            );
        }
    }

    /// Reasoning the record keeps in clear; an empty or redacted thought
    /// draws nothing.
    pub fn thought(&mut self, message: Option<&str>, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let block = text_block(text, None);
        self.chunk(
            SessionUpdate::AgentThoughtChunk,
            message,
            Origin::Operator,
            block,
        );
    }

    pub fn call(&mut self, call: Call) {
        let locations = call
            .paths
            .iter()
            .map(|path| ToolCallLocation::new(wire(path)))
            .collect::<Vec<_>>();
        let content = call.diffs.into_iter().map(diff_content).collect::<Vec<_>>();
        if !self.called.insert(call.id.clone()) {
            let fields = ToolCallUpdateFields::new()
                .title(call.title)
                .kind(call.kind)
                .locations(locations)
                .raw_input(call.input);
            let fields = if content.is_empty() {
                fields
            } else {
                fields.content(content)
            };
            self.updates
                .push(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    call.id, fields,
                )));
            return;
        }
        self.updates.push(SessionUpdate::ToolCall(
            ToolCall::new(call.id, call.title)
                .name(call.name)
                .kind(call.kind)
                .status(call.status)
                .locations(locations)
                .content(content)
                .raw_input(call.input),
        ));
    }

    /// A call whose result will follow it in order, with no id of its own.
    pub fn call_awaiting_result(&mut self, call: Call) {
        self.awaiting.push_back(call.id.clone());
        self.call(call);
    }

    /// The next result in order belongs to the oldest call still waiting;
    /// with none waiting, the result is passed over and counted.
    pub fn result_in_order(&mut self, outcome: Outcome<'_>) {
        match self.awaiting.pop_front() {
            Some(id) => self.result(&id, outcome),
            None => self.skip("result_without_call"),
        }
    }

    /// Calls that never met their result keep their call alone (B8).
    pub fn forget_awaiting(&mut self) {
        self.awaiting.clear();
    }

    pub fn result(&mut self, id: &str, outcome: Outcome<'_>) {
        let mut content = outcome
            .diffs
            .into_iter()
            .map(diff_content)
            .collect::<Vec<_>>();
        if let Some(output) = outcome.output.filter(|output| !output.is_empty()) {
            content.push(ToolCallContent::Content(Content::new(text_block(
                &output, None,
            ))));
        }
        content.extend(
            outcome
                .images
                .into_iter()
                .filter_map(media_block)
                .map(|block| ToolCallContent::Content(Content::new(block))),
        );
        let status = if outcome.failed {
            ToolCallStatus::Failed
        } else if outcome.running {
            ToolCallStatus::InProgress
        } else {
            ToolCallStatus::Completed
        };
        // An update's content replaces the call's, so a result that says
        // nothing more keeps the diff or output the call already showed.
        let fields = ToolCallUpdateFields::new().status(status);
        let fields = if content.is_empty() {
            fields
        } else {
            fields.content(content)
        };
        self.updates
            .push(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                id.to_owned(),
                fields,
            )));
    }

    pub fn plan(&mut self, steps: impl IntoIterator<Item = Step>) {
        let entries = steps
            .into_iter()
            .map(|step| {
                let (text, truncated) = capped(&step.text);
                let entry = PlanEntry::new(text, PlanEntryPriority::Medium, step.status);
                if truncated {
                    entry.meta(hide_meta([("truncated", json!(true))]))
                } else {
                    entry
                }
            })
            .collect();
        self.updates.push(SessionUpdate::Plan(Plan::new(entries)));
    }

    fn chunk(
        &mut self,
        update: fn(ContentChunk) -> SessionUpdate,
        message: Option<&str>,
        origin: Origin,
        block: ContentBlock,
    ) {
        let chunk = ContentChunk::new(block).message_id(message.map(str::to_owned).map(Into::into));
        let chunk = match origin.as_str() {
            Some(origin) => chunk.meta(hide_meta([("origin", json!(origin))])),
            None => chunk,
        };
        self.updates.push(update(chunk));
    }
}

/// What the text itself says the message is, unless metadata already said.
fn classify(origin: Origin, text: &str) -> Origin {
    if origin != Origin::Operator {
        origin
    } else if crate::is_interruption(text) {
        Origin::Interrupted
    } else if crate::is_letter(text) {
        Origin::Letter
    } else if crate::has_injected_prefix(text) {
        Origin::Injected
    } else {
        Origin::Operator
    }
}

const PASTED_OPEN: &str = "<pasted_content";
const PASTED_CLOSE: &str = "</pasted_content>";

/// `text` cut at each pasted-content wrapper, the wrapped text marked as
/// pasted and without its tags. An unclosed wrapper is the operator's text.
fn pasted_pieces(text: &str) -> Vec<(&str, bool)> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(PASTED_OPEN) {
        let Some(tag_end) = rest[open..].find('>').map(|end| open + end + 1) else {
            break;
        };
        let Some(close) = rest[tag_end..].find(PASTED_CLOSE).map(|end| tag_end + end) else {
            break;
        };
        pieces.push((&rest[..open], false));
        pieces.push((&rest[tag_end..close], true));
        rest = &rest[close + PASTED_CLOSE.len()..];
    }
    pieces.push((rest, false));
    pieces.retain(|(piece, _)| !piece.trim().is_empty());
    pieces
}

fn text_block(text: &str, mark: Option<(&'static str, Value)>) -> ContentBlock {
    let (text, truncated) = capped(text);
    let marks = mark
        .into_iter()
        .chain(truncated.then(|| ("truncated", json!(true))))
        .collect::<Vec<_>>();
    let block = TextContent::new(text);
    ContentBlock::Text(if marks.is_empty() {
        block
    } else {
        block.meta(hide_meta(marks))
    })
}

fn media_block(part: Part<'_>) -> Option<ContentBlock> {
    match part {
        Part::Text(text) => Some(text_block(text, None)),
        // A picture cut short is no picture: past the cap the block keeps
        // its type and size and says it was left out (B10).
        Part::Image { data, mime } if data.len() > IMAGE_LIMIT_BYTES => Some(ContentBlock::Image(
            ImageContent::new(String::new(), mime).meta(hide_meta([
                ("truncated", json!(true)),
                ("bytes", json!(data.len())),
            ])),
        )),
        Part::Image { data, mime } => Some(ContentBlock::Image(ImageContent::new(data, mime))),
        Part::ImagePath { path, mime } => {
            let path = wire(&path);
            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
            let mime = mime.map(str::to_owned).or_else(|| image_mime(&path));
            Some(ContentBlock::ResourceLink(
                ResourceLink::new(name, format!("file://{}", file_uri_path(&path))).mime_type(mime),
            ))
        }
    }
}

fn diff_content(diff: FileDiff) -> ToolCallContent {
    let (after, after_cut) = capped(&diff.after);
    let (before, before_cut) = match diff.before.as_deref().map(capped) {
        Some((before, cut)) => (Some(before), cut),
        None => (None, false),
    };
    let content = Diff::new(wire(&diff.path), after).old_text(before);
    ToolCallContent::Diff(if after_cut || before_cut {
        content.meta(hide_meta([("truncated", json!(true))]))
    } else {
        content
    })
}

/// `text` within [`TEXT_LIMIT_CHARS`], and whether it was cut.
fn capped(text: &str) -> (String, bool) {
    match text.char_indices().nth(TEXT_LIMIT_CHARS) {
        Some((end, _)) => (text[..end].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

fn hide_meta(marks: impl IntoIterator<Item = (&'static str, Value)>) -> Meta {
    let marks = marks
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<serde_json::Map<_, _>>();
    Meta::from_iter([("hide".to_owned(), Value::Object(marks))])
}

/// A record's path in the `/` form every system reads (B18). A relative
/// path, such as one a tool reports against its cwd, keeps its words.
fn wire(path: &str) -> String {
    let native = std::path::Path::new(path);
    if native.is_absolute() {
        hide_platform::path::to_wire_lossy(native)
    } else {
        path.replace('\\', "/")
    }
}

/// A wire path as a `file:` URI path: a drive path gains the leading `/`.
fn file_uri_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn image_mime(path: &str) -> Option<String> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    Some(mime.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_content_is_cut_out_of_the_typed_words() {
        let text = "look at this\n<pasted_content id=\"a1\">\nlog line\n</pasted_content>\nwhy?";
        assert_eq!(
            pasted_pieces(text),
            vec![
                ("look at this\n", false),
                ("\nlog line\n", true),
                ("\nwhy?", false)
            ]
        );
        assert_eq!(
            pasted_pieces("<pasted_content id=\"a\">never closed"),
            vec![("<pasted_content id=\"a\">never closed", false)]
        );
    }

    #[test]
    fn text_at_the_cap_is_whole_and_one_past_it_is_marked() {
        let at = "가".repeat(TEXT_LIMIT_CHARS);
        assert_eq!(capped(&at), (at.clone(), false));
        let past = format!("{at}나");
        assert_eq!(capped(&past), (at, true));
    }

    #[test]
    fn an_image_past_the_cap_keeps_its_type_and_drops_its_data() {
        let data = "A".repeat(IMAGE_LIMIT_BYTES + 1);
        let Some(ContentBlock::Image(image)) = media_block(Part::Image {
            data: &data,
            mime: "image/png",
        }) else {
            panic!("an image block");
        };
        assert!(image.data.is_empty());
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.meta.unwrap()["hide"]["truncated"], json!(true));
    }

    #[test]
    fn order_pairing_gives_each_result_to_the_oldest_waiting_call() {
        let mut thread = Thread::default();
        thread.call_awaiting_result(Call::new("a", "run", ToolKind::Execute));
        thread.call_awaiting_result(Call::new("b", "read", ToolKind::Read));
        thread.result_in_order(Outcome::default());
        thread.forget_awaiting();
        thread.result_in_order(Outcome::default());
        let (updates, skipped) = thread.into_parts();
        let results = updates
            .iter()
            .filter_map(|update| match update {
                SessionUpdate::ToolCallUpdate(update) => Some(update.tool_call_id.0.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results, ["a"]);
        assert_eq!(skipped.get("result_without_call"), Some(&1));
    }
}
