// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/detect/manifest.rs and the `ManifestVersion` of src/detect/manifest_update.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: the rule grammar, its limits, its compiler, its evaluator
// and the screen regions only. The rules are a value a caller hands in (an
// agent definition's `[screen]` table) instead of a process-wide cache of
// bundled, downloaded and override files, so the remote catalog, the cache,
// the override path, the explain report and Herdr's agent table are gone; a
// manifest's `version` and `min_engine_version` are required; a rule set no
// rule of which matches answers `None` instead of Herdr's idle fallback, which
// the caller decides.

//! Screen rules: how an agent's state is read from what its terminal shows.
//!
//! A rule set is the `[screen]` table of an agent definition file, in the
//! grammar of Herdr's agent detection manifests: each rule names a state, a
//! priority, a region of the screen and the matchers (`contains`, `regex`,
//! `line_regex`, nested `all`, `any` and `not` gates) that must hold there.
//! [`Manifest::compile`] checks a set against the limits below and this
//! evaluator's version and compiles its patterns once; [`Rules::evaluate`]
//! answers the highest-priority rule that matches a screen.
//!
//! Rust's `regex` runs in time linear in the text, and the limits bound the
//! work a rule set can ask for, so a rule set from the operator's own file is
//! as safe to evaluate as a built-in one.

use std::cmp::Ordering;
use std::fmt;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// The version of this evaluator: the regions and matchers it knows. A rule
/// set whose `min_engine_version` is higher was written for a newer one.
/// It is Herdr v0.9.5's `MANIFEST_ENGINE_VERSION`, whose rules this reads.
pub const ENGINE_VERSION: u32 = 3;

/// The most rules one set may hold.
pub const MAX_RULES: usize = 128;
/// The deepest a gate may nest.
pub const MAX_GATE_DEPTH: usize = 8;
/// The most gates one set may hold, rules included.
pub const MAX_TOTAL_GATES: usize = 512;
/// The most direct matchers one gate may hold.
pub const MAX_MATCHERS_PER_GATE: usize = 32;
/// The most matchers one set may hold.
pub const MAX_TOTAL_MATCHERS: usize = 1024;
/// The longest one matcher may be, in characters.
pub const MAX_MATCHER_CHARS: usize = 512;

/// The `top_non_empty_lines(n)` region arrived with engine 3.
const TOP_NON_EMPTY_LINES_ENGINE_VERSION: u32 = 3;
const MAX_TOP_REGION_LINE_COUNT: usize = u16::MAX as usize;

/// An agent state a rule can name.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl State {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        }
    }
}

/// A rule set's version: dotted numbers (`2026.10.11.1`), compared number
/// by number, a missing trailing number reading as zero.
#[derive(Debug, Clone)]
pub struct Version(String);

impl Version {
    pub fn parse(value: &str) -> Result<Self, String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("version must not be empty".to_owned());
        }
        for segment in trimmed.split('.') {
            if segment.is_empty() {
                return Err(format!("version {trimmed:?} contains an empty segment"));
            }
            if !segment.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(format!("version {trimmed:?} must be dotted numeric"));
            }
            segment
                .parse::<u64>()
                .map_err(|_| format!("version {trimmed:?} contains an oversized segment"))?;
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // Every segment parsed as a u64 in `parse`.
        let number = |segment: &str| segment.parse::<u64>().unwrap_or(0);
        let mut left = self.0.split('.');
        let mut right = other.0.split('.');
        loop {
            match (left.next(), right.next()) {
                (Some(left), Some(right)) => match number(left).cmp(&number(right)) {
                    Ordering::Equal => {}
                    ordering => return ordering,
                },
                (Some(left), None) => {
                    if number(left) != 0 {
                        return Ordering::Greater;
                    }
                }
                (None, Some(right)) => {
                    if number(right) != 0 {
                        return Ordering::Less;
                    }
                }
                (None, None) => return Ordering::Equal,
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

/// A rule set as written: an agent definition's `[screen]` table. It
/// serializes to what it reads back, so a rule set can be handed to another
/// process (a pane's holder) and compiled there.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: Version,
    pub min_engine_version: u32,
    /// When the rules were last changed, as their author wrote it; not read.
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub state: Option<State>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub visible_idle: bool,
    #[serde(default)]
    pub visible_blocker: bool,
    #[serde(default)]
    pub visible_working: bool,
    #[serde(default)]
    pub skip_state_update: bool,
    #[serde(default)]
    all: Vec<Gate>,
    #[serde(default)]
    any: Vec<Gate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<Gate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Gate {
    #[serde(default)]
    all: Vec<Gate>,
    #[serde(default)]
    any: Vec<Gate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<Gate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

fn default_region() -> String {
    "whole_recent".to_owned()
}

/// A rule set checked and compiled: what a screen is evaluated against.
#[derive(Debug, Clone)]
pub struct Rules {
    manifest: Manifest,
    compiled: Vec<CompiledGate>,
}

#[derive(Debug, Clone)]
struct CompiledGate {
    all: Vec<CompiledGate>,
    any: Vec<CompiledGate>,
    not_gate: Vec<CompiledGate>,
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

/// What a screen is read from: its text, and the strings the terminal's OSC
/// title and progress sequences last set (empty when none was).
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub screen: &'a str,
    pub osc_title: &'a str,
    pub osc_progress: &'a str,
}

impl<'a> Input<'a> {
    /// A screen with no OSC title or progress.
    pub fn screen(screen: &'a str) -> Self {
        Self {
            screen,
            osc_title: "",
            osc_progress: "",
        }
    }
}

/// The rule a screen matched, the highest priority among those that did
/// (the first written of equal priorities).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection<'r> {
    pub rule: &'r str,
    pub state: State,
    /// The rule says the state is plain on the screen, not inferred.
    pub visible: bool,
    /// The rule says the screen proves nothing: keep the state as it was.
    pub skip_state_update: bool,
}

impl Manifest {
    /// Check the set against the limits and this evaluator's version, then
    /// compile every pattern once. The error names the rule and the reason.
    pub fn compile(self) -> Result<Rules, String> {
        if self.min_engine_version > ENGINE_VERSION {
            return Err(format!(
                "rules require engine {}, this one is {ENGINE_VERSION}",
                self.min_engine_version
            ));
        }
        validate(&self)?;
        let compiled = self
            .rules
            .iter()
            .map(|rule| {
                compile_gate(&rule_gate(rule))
                    .map_err(|error| format!("rule {} could not be compiled: {error}", rule.id))
            })
            .collect::<Result<_, _>>()?;
        Ok(Rules {
            manifest: self,
            compiled,
        })
    }
}

impl Rules {
    pub fn version(&self) -> &Version {
        &self.manifest.version
    }

    pub fn min_engine_version(&self) -> u32 {
        self.manifest.min_engine_version
    }

    pub fn rules(&self) -> &[Rule] {
        &self.manifest.rules
    }

    /// The rule set as it was written, to hand to another process.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The highest-priority rule that matches, or `None` when none does.
    pub fn evaluate(&self, input: Input<'_>) -> Option<Detection<'_>> {
        let mut matched: Option<&Rule> = None;
        for (rule, gate) in self.manifest.rules.iter().zip(&self.compiled) {
            if matched.is_some_and(|previous| previous.priority >= rule.priority) {
                continue;
            }
            let text = region(input, &rule.region);
            if gate_matches(gate, text, &text.to_lowercase()) {
                matched = Some(rule);
            }
        }
        matched.map(|rule| {
            let state = rule.state.unwrap_or(State::Unknown);
            Detection {
                rule: &rule.id,
                state,
                visible: match state {
                    State::Idle => rule.visible_idle,
                    State::Blocked => rule.visible_blocker,
                    State::Working => rule.visible_working,
                    State::Unknown => false,
                },
                skip_state_update: rule.skip_state_update,
            }
        })
    }
}

fn validate(manifest: &Manifest) -> Result<(), String> {
    if manifest.rules.is_empty() {
        return Err("rules must contain at least one rule".to_owned());
    }
    if manifest.rules.len() > MAX_RULES {
        return Err(format!(
            "rules contain {} rules, at most {MAX_RULES}",
            manifest.rules.len()
        ));
    }
    let mut complexity = Complexity::default();
    for rule in &manifest.rules {
        if rule.id.trim().is_empty() {
            return Err("a rule id must not be empty".to_owned());
        }
        if rule.skip_state_update {
            if rule.state != Some(State::Unknown) {
                return Err(format!(
                    "rule {} uses skip_state_update without state = \"unknown\"",
                    rule.id
                ));
            }
            if rule.visible_idle || rule.visible_blocker || rule.visible_working {
                return Err(format!(
                    "rule {} uses skip_state_update with visible state evidence",
                    rule.id
                ));
            }
        }
        validate_region(&rule.region)
            .map_err(|region| format!("rule {} uses unknown region {region}", rule.id))?;
        if rule.region.trim().starts_with("top_non_empty_lines(")
            && manifest.min_engine_version < TOP_NON_EMPTY_LINES_ENGINE_VERSION
        {
            return Err(format!(
                "rule {} uses top_non_empty_lines but min_engine_version is below {TOP_NON_EMPTY_LINES_ENGINE_VERSION}",
                rule.id
            ));
        }
        validate_gate(&rule_gate(rule), "rule", 0, &mut complexity)
            .map_err(|error| format!("rule {} has invalid matcher gates: {error}", rule.id))?;
    }
    Ok(())
}

#[derive(Default)]
struct Complexity {
    gates: usize,
    matchers: usize,
}

fn validate_gate(
    gate: &Gate,
    context: &str,
    depth: usize,
    complexity: &mut Complexity,
) -> Result<(), String> {
    count_gate(gate, context, depth, complexity)?;
    if !has_positive_matcher(gate) {
        return Err(format!("{context} must contain a positive matcher"));
    }
    validate_nested(gate, context, depth, complexity)
}

fn validate_not_gate(gate: &Gate, depth: usize, complexity: &mut Complexity) -> Result<(), String> {
    count_gate(gate, "not gate", depth, complexity)?;
    if !has_any_matcher(gate) {
        return Err("not gate must contain a matcher".to_owned());
    }
    validate_nested(gate, "not gate", depth, complexity)
}

fn count_gate(
    gate: &Gate,
    context: &str,
    depth: usize,
    complexity: &mut Complexity,
) -> Result<(), String> {
    if depth > MAX_GATE_DEPTH {
        return Err(format!("{context} exceeds max gate depth {MAX_GATE_DEPTH}"));
    }
    complexity.gates += 1;
    if complexity.gates > MAX_TOTAL_GATES {
        return Err(format!("rules exceed max gate count {MAX_TOTAL_GATES}"));
    }
    let direct = gate.contains.len() + gate.regex.len() + gate.line_regex.len();
    if direct > MAX_MATCHERS_PER_GATE {
        return Err(format!(
            "{context} has {direct} direct matchers, at most {MAX_MATCHERS_PER_GATE}"
        ));
    }
    complexity.matchers += direct;
    if complexity.matchers > MAX_TOTAL_MATCHERS {
        return Err(format!(
            "rules exceed max matcher count {MAX_TOTAL_MATCHERS}"
        ));
    }
    if gate
        .contains
        .iter()
        .chain(&gate.regex)
        .chain(&gate.line_regex)
        .any(|value| value.chars().count() > MAX_MATCHER_CHARS)
    {
        return Err(format!(
            "{context} matcher exceeds max length {MAX_MATCHER_CHARS}"
        ));
    }
    for (patterns, field) in [(&gate.regex, "regex"), (&gate.line_regex, "line_regex")] {
        for pattern in patterns {
            Regex::new(pattern).map_err(|error| {
                format!("{context} contains invalid {field} pattern {pattern:?}: {error}")
            })?;
        }
    }
    Ok(())
}

fn validate_nested(
    gate: &Gate,
    context: &str,
    depth: usize,
    complexity: &mut Complexity,
) -> Result<(), String> {
    let negated = context.starts_with("not");
    for nested in &gate.all {
        let name = if negated { "not all gate" } else { "all gate" };
        validate_gate(nested, name, depth + 1, complexity)?;
    }
    for nested in &gate.any {
        let name = if negated { "not any gate" } else { "any gate" };
        validate_gate(nested, name, depth + 1, complexity)?;
    }
    for nested in &gate.not_gate {
        if !negated && !has_any_matcher(nested) {
            return Err(format!("{context} contains an empty not gate"));
        }
        validate_not_gate(nested, depth + 1, complexity)?;
    }
    Ok(())
}

fn has_positive_matcher(gate: &Gate) -> bool {
    !gate.contains.is_empty()
        || !gate.regex.is_empty()
        || !gate.line_regex.is_empty()
        || !gate.all.is_empty()
        || !gate.any.is_empty()
}

fn has_any_matcher(gate: &Gate) -> bool {
    has_positive_matcher(gate) || !gate.not_gate.is_empty()
}

fn validate_region(spec: &str) -> Result<(), String> {
    let trimmed = spec.trim();
    match trimmed {
        "whole_recent"
        | "after_last_prompt_marker"
        | "before_current_prompt_marker"
        | "whole_recent_without_current_prompt_marker"
        | "current_prompt_block_marker"
        | "after_current_prompt_block_marker"
        | "prompt_box_body"
        | "above_prompt_box"
        | "last_non_empty_above_prompt_box"
        | "after_last_horizontal_rule"
        | "osc_title"
        | "osc_progress" => Ok(()),
        _ if region_count(trimmed, "bottom_lines").is_some()
            || region_count(trimmed, "bottom_non_empty_lines").is_some()
            || top_region_count(trimmed).is_some() =>
        {
            Ok(())
        }
        _ => Err(trimmed.to_owned()),
    }
}

fn rule_gate(rule: &Rule) -> Gate {
    Gate {
        all: rule.all.clone(),
        any: rule.any.clone(),
        not_gate: rule.not_gate.clone(),
        contains: rule.contains.clone(),
        regex: rule.regex.clone(),
        line_regex: rule.line_regex.clone(),
    }
}

fn compile_gate(gate: &Gate) -> Result<CompiledGate, String> {
    let patterns = |patterns: &[String]| {
        patterns
            .iter()
            .map(|pattern| Regex::new(pattern).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()
    };
    let gates = |gates: &[Gate]| {
        gates
            .iter()
            .map(compile_gate)
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(CompiledGate {
        all: gates(&gate.all)?,
        any: gates(&gate.any)?,
        not_gate: gates(&gate.not_gate)?,
        contains: gate
            .contains
            .iter()
            .map(|needle| needle.to_lowercase())
            .collect(),
        regex: patterns(&gate.regex)?,
        line_regex: patterns(&gate.line_regex)?,
    })
}

fn gate_matches(gate: &CompiledGate, text: &str, lower_text: &str) -> bool {
    gate.contains
        .iter()
        .all(|needle| lower_text.contains(needle))
        && gate.regex.iter().all(|regex| regex.is_match(text))
        && gate
            .line_regex
            .iter()
            .all(|regex| text.lines().any(|line| regex.is_match(line)))
        && gate
            .all
            .iter()
            .all(|nested| gate_matches(nested, text, lower_text))
        && (gate.any.is_empty()
            || gate
                .any
                .iter()
                .any(|nested| gate_matches(nested, text, lower_text)))
        && !gate
            .not_gate
            .iter()
            .any(|nested| gate_matches(nested, text, lower_text))
}

fn region<'a>(input: Input<'a>, spec: &str) -> &'a str {
    let trimmed = spec.trim();
    match trimmed {
        "osc_title" => return input.osc_title,
        "osc_progress" => return input.osc_progress,
        _ => {}
    }
    let content = input.screen;
    match trimmed {
        "whole_recent" => content,
        "after_last_prompt_marker" => after_last_prompt_marker(content),
        "before_current_prompt_marker" => before_current_prompt_marker(content),
        "whole_recent_without_current_prompt_marker" => {
            whole_recent_without_current_prompt_marker(content)
        }
        "current_prompt_block_marker" => current_prompt_block_marker(content).unwrap_or(""),
        "after_current_prompt_block_marker" => {
            after_current_prompt_block_marker(content).unwrap_or("")
        }
        "prompt_box_body" => prompt_box_body(content).unwrap_or(""),
        "above_prompt_box" => above_prompt_box(content),
        "last_non_empty_above_prompt_box" => last_non_empty_line(above_prompt_box(content)),
        "after_last_horizontal_rule" => after_last_horizontal_rule(content),
        _ => {
            if let Some(count) = region_count(trimmed, "bottom_lines") {
                return bottom_lines(content, count);
            }
            if let Some(count) = region_count(trimmed, "bottom_non_empty_lines") {
                return bottom_non_empty_lines(content, count);
            }
            if let Some(count) = top_region_count(trimmed) {
                return top_non_empty_lines(content, count);
            }
            ""
        }
    }
}

fn region_count(spec: &str, name: &str) -> Option<usize> {
    spec.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|count| count.parse::<usize>().ok())
}

fn top_region_count(spec: &str) -> Option<usize> {
    let count = spec
        .strip_prefix("top_non_empty_lines")?
        .strip_prefix('(')?
        .strip_suffix(')')?;
    if count.starts_with('0') || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    count
        .parse::<usize>()
        .ok()
        .filter(|count| *count <= MAX_TOP_REGION_LINE_COUNT)
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(count);
    slice_from_line_index(content, &lines, start)
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(start_index) = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    slice_from_line_index(content, &lines, start_index)
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(end_index) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    let byte_offset = line_start_offset(content, &lines, end_index + 1);
    &content[..byte_offset]
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(index) = lines.iter().rposition(|line| codex_prompt_line(line)) else {
        return content;
    };
    slice_from_line_index(content, &lines, index + 1)
}

fn before_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(index) = current_codex_prompt_index(&lines) else {
        return content;
    };
    let byte_offset = lines[..index]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    &content[..byte_offset.min(content.len())]
}

fn whole_recent_without_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    if current_codex_prompt_index(&lines).is_some() {
        ""
    } else {
        content
    }
}

fn current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let prompt_index = current_codex_prompt_index(&lines)?;
    lines[..prompt_index]
        .iter()
        .rev()
        .find(|line| codex_block_marker_line(line))
        .copied()
}

fn after_current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let prompt_index = current_codex_prompt_index(&lines)?;
    let block_index = lines[..prompt_index]
        .iter()
        .rposition(|line| codex_block_marker_line(line))?;
    Some(slice_from_line_index(content, &lines, block_index))
}

fn current_codex_prompt_index(lines: &[&str]) -> Option<usize> {
    let prompt_index = lines.iter().rposition(|line| codex_prompt_line(line))?;
    if lines[prompt_index + 1..]
        .iter()
        .any(|line| codex_block_marker_line(line))
    {
        return None;
    }
    Some(prompt_index)
}

fn codex_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn codex_block_marker_line(line: &str) -> bool {
    line.starts_with('•') || line.starts_with('■') || line.starts_with('✗') || line.starts_with('✓')
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let top = prompt_box_top_border_index(&lines)?;
    let start = line_start_offset(content, &lines, top + 1);
    let end_index = lines[top + 1..]
        .iter()
        .position(|line| is_horizontal_rule(line))
        .map(|relative| top + 1 + relative)
        .unwrap_or(lines.len());
    let end = line_start_offset(content, &lines, end_index);
    Some(&content[start.min(content.len())..end.min(content.len())])
}

fn above_prompt_box(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(top) = prompt_box_top_border_index(&lines) else {
        return content;
    };
    let end = line_start_offset(content, &lines, top);
    &content[..end.min(content.len())]
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let mut last_rule_end = 0usize;
    let mut offset = 0usize;
    for line in content.lines() {
        let next_offset = offset + line.len() + 1;
        if is_horizontal_rule(line) {
            last_rule_end = next_offset.min(content.len());
        }
        offset = next_offset;
    }
    &content[last_rule_end..]
}

fn last_non_empty_line(content: &str) -> &str {
    content
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}

fn prompt_box_top_border_index(lines: &[&str]) -> Option<usize> {
    let mut border_count = 0;
    for index in (0..lines.len()).rev() {
        if is_horizontal_rule(lines[index]) {
            border_count += 1;
            if border_count == 2 {
                return Some(index);
            }
        }
    }
    None
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let rule_chars = trimmed.chars().take_while(|&ch| ch == '─').count();
    if rule_chars == 0 {
        return false;
    }
    let rule_bytes = trimmed
        .char_indices()
        .nth(rule_chars)
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let suffix = trimmed[rule_bytes..].trim_start();
    suffix.is_empty() || rule_chars >= 3
}

fn slice_from_line_index<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    let byte_offset = line_start_offset(content, lines, index);
    &content[byte_offset.min(content.len())..]
}

fn line_start_offset(content: &str, lines: &[&str], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(content.len())
}
