//! `hide issue help`: the guide for reporting a bug in Hide itself, which the
//! `hide-issue` skill points at. It prints text and needs no daemon, no
//! environment and no network; the report itself is filed by the agent with
//! `gh`, after the operator agrees.

/// What `hide issue help` prints: when to report, what to collect, the bug
/// form's template and who says yes before it is filed.
pub const HELP: &str = include_str!("../assets/issue/help.md");

/// The one command of the topic.
pub const USAGE: &str = "hide issue help";

#[cfg(test)]
mod tests {
    use super::*;

    /// The template names the labels of the repository's bug form, so a
    /// renamed field fails here rather than in an agent's report.
    #[test]
    fn the_template_headings_are_labels_of_the_bug_form() {
        let form = include_str!("../../.github/ISSUE_TEMPLATE/bug.yml");
        let labels: Vec<&str> = form
            .lines()
            .filter_map(|line| line.trim().strip_prefix("label: "))
            .collect();
        let template = HELP
            .split("## Template")
            .nth(1)
            .expect("a template section");
        let headings: Vec<&str> = template
            .lines()
            .filter_map(|line| line.strip_prefix("### "))
            .collect();
        assert!(headings.len() >= 8, "{headings:?}");
        for heading in headings {
            assert!(
                labels.contains(&heading),
                "{heading} is not a bug form label: {labels:?}"
            );
        }
    }

    #[test]
    fn the_guide_keeps_the_rules_that_protect_the_operator() {
        for rule in [
            "gh issue list --repo with-hide/hide",
            "Logs/core.jsonl",
            "Do not add a root cause, a proposed fix or a patch",
            "wait for their yes",
            "gh issue create --repo with-hide/hide",
            "issues/new?template=bug.yml",
            "Keep it under 6,000 characters",
        ] {
            assert!(HELP.contains(rule), "{rule}");
        }
    }
}
