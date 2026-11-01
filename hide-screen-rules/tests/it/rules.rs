use hide_screen_rules::{ENGINE_VERSION, Input, Manifest, Rules, State, Version};

fn compile(rules: &str) -> Result<Rules, String> {
    compile_at(1, rules)
}

fn compile_at(engine: u32, rules: &str) -> Result<Rules, String> {
    let text = format!("version = \"1\"\nmin_engine_version = {engine}\n{rules}");
    let manifest: Manifest = toml::from_str(&text).map_err(|error| error.to_string())?;
    manifest.compile()
}

fn state(rules: &Rules, screen: &str) -> Option<State> {
    rules
        .evaluate(Input::screen(screen))
        .map(|detection| detection.state)
}

#[test]
fn the_highest_priority_matching_rule_wins_through_nested_gates() {
    let rules = compile(
        r#"
[[rules]]
id = "low_contains"
state = "idle"
priority = 1
contains = ["match"]

[[rules]]
id = "high_nested_gates"
state = "working"
priority = 10
contains = ["match"]
all = [
  { any = [{ regex = ["w[io]n"] }, { contains = ["fallback"] }] },
]
not = [
  { contains = ["blocked"] },
]

[[rules]]
id = "line_regex"
state = "blocked"
priority = 20
line_regex = ["^exact line$"]
"#,
    )
    .unwrap();
    let high = rules.evaluate(Input::screen("match win")).unwrap();
    assert_eq!(
        (high.rule, high.state),
        ("high_nested_gates", State::Working)
    );
    assert_eq!(state(&rules, "MATCH FALLBACK"), Some(State::Working));
    assert_eq!(state(&rules, "match win blocked"), Some(State::Idle));
    assert_eq!(state(&rules, "match\nexact line"), Some(State::Blocked));
    assert_eq!(state(&rules, "an exact line"), None);
}

#[test]
fn equal_priorities_keep_the_first_rule_written() {
    let rules = compile(
        r#"
[[rules]]
id = "first"
state = "idle"
contains = ["ready"]

[[rules]]
id = "second"
state = "working"
contains = ["ready"]
"#,
    )
    .unwrap();
    assert_eq!(
        rules.evaluate(Input::screen("ready")).unwrap().rule,
        "first"
    );
}

#[test]
fn paired_alternatives_require_complete_gates_at_the_bottom() {
    let rules = compile(
        r#"
[[rules]]
id = "paired"
state = "blocked"
region = "bottom_non_empty_lines(3)"
visible_blocker = true
any = [
  { all = [
    { contains = ["alpha heading"] },
    { regex = ['alpha\s+control\s*\z'] },
  ] },
  { all = [
    { contains = ["beta heading"] },
    { regex = ['beta\s+control\s*\z'] },
  ] },
]
"#,
    )
    .unwrap();
    for text in [
        "alpha heading\nalpha control",
        "beta heading\nbeta\ncontrol",
    ] {
        let detection = rules.evaluate(Input::screen(text)).unwrap();
        assert_eq!(detection.state, State::Blocked);
        assert!(detection.visible);
    }
    for text in [
        "alpha heading\nbeta control",
        "beta heading\nalpha control",
        "beta control",
        "beta heading",
        "beta heading\nbeta control\nnewer content",
        "beta heading\nfiller one\nfiller two\nbeta control",
    ] {
        assert_eq!(state(&rules, text), None, "{text}");
    }
}

#[test]
fn osc_regions_read_their_own_inputs_and_share_rule_priority() {
    let rules = compile(
        r#"
[[rules]]
id = "screen"
state = "idle"
priority = 10
visible_idle = true
contains = ["screen-marker"]

[[rules]]
id = "title"
state = "working"
priority = 20
region = "osc_title"
regex = ['^title-marker$']

[[rules]]
id = "progress"
state = "blocked"
priority = 30
region = "osc_progress"
regex = ['^progress-marker$']
"#,
    )
    .unwrap();
    for (screen, osc_title, osc_progress, rule) in [
        ("screen-marker", "", "", "screen"),
        ("screen-marker", "title-marker", "", "title"),
        (
            "screen-marker",
            "title-marker",
            "progress-marker",
            "progress",
        ),
        (
            "screen-marker title-marker progress-marker",
            "",
            "",
            "screen",
        ),
    ] {
        let input = Input {
            screen,
            osc_title,
            osc_progress,
        };
        assert_eq!(rules.evaluate(input).unwrap().rule, rule);
    }
    let swapped = Input {
        screen: "",
        osc_title: "progress-marker",
        osc_progress: "title-marker",
    };
    assert!(rules.evaluate(swapped).is_none());
}

#[test]
fn a_skip_rule_says_the_screen_proves_nothing() {
    let rules = compile(
        r#"
[[rules]]
id = "menu"
state = "unknown"
skip_state_update = true
contains = ["menu"]
"#,
    )
    .unwrap();
    let detection = rules.evaluate(Input::screen("a menu")).unwrap();
    assert!(detection.skip_state_update);
    assert!(!detection.visible);
}

#[test]
fn prompt_box_regions_split_the_screen_at_its_rules() {
    let rules = compile(
        r#"
[[rules]]
id = "inside"
state = "idle"
region = "prompt_box_body"
line_regex = ['^❯']

[[rules]]
id = "above"
state = "working"
priority = 5
region = "above_prompt_box"
contains = ["thinking"]
"#,
    )
    .unwrap();
    let box_screen = "output\n────────\n❯ \n────────\nfooter";
    assert_eq!(state(&rules, box_screen), Some(State::Idle));
    let thinking = "thinking\n────────\n❯ \n────────\n";
    assert_eq!(state(&rules, thinking), Some(State::Working));
    assert_eq!(state(&rules, "❯ no box"), None);
}

#[test]
fn rule_sets_that_break_the_grammar_or_its_limits_are_refused_with_the_rule_named() {
    let refused = [
        (
            "typo",
            "[[rules]]\nid = \"typo\"\nstate = \"working\"\ncontain = [\"Working\"]\n",
        ),
        ("empty", "[[rules]]\nid = \"empty\"\nstate = \"working\"\n"),
        (
            "bad_region",
            "[[rules]]\nid = \"bad_region\"\nstate = \"working\"\nregion = \"after_last_promt_marker\"\ncontains = [\"x\"]\n",
        ),
        (
            "bad_regex",
            "[[rules]]\nid = \"bad_regex\"\nstate = \"working\"\nregex = [\"[\"]\n",
        ),
        (
            "bad_nested_regex",
            "[[rules]]\nid = \"bad_nested_regex\"\nstate = \"working\"\nany = [{ line_regex = [\"[\"] }]\n",
        ),
        (
            "bad_skip_state",
            "[[rules]]\nid = \"bad_skip_state\"\nstate = \"idle\"\nskip_state_update = true\ncontains = [\"menu\"]\n",
        ),
        (
            "bad_skip_visible",
            "[[rules]]\nid = \"bad_skip_visible\"\nstate = \"unknown\"\nskip_state_update = true\nvisible_blocker = true\ncontains = [\"menu\"]\n",
        ),
    ];
    for (name, rules) in refused {
        let error = compile(rules).expect_err(name);
        // A field the grammar does not know is refused by the parser; every
        // other refusal names its rule.
        assert!(name == "typo" || error.contains(name), "{name}: {error}");
    }
    assert!(compile("").is_err(), "a set with no rule");
}

#[test]
fn rule_sets_past_a_limit_are_refused() {
    let many_rules: String = (0..=hide_screen_rules::MAX_RULES)
        .map(|index| {
            format!("[[rules]]\nid = \"r{index}\"\nstate = \"idle\"\ncontains = [\"ready\"]\n")
        })
        .collect();
    assert!(compile(&many_rules).unwrap_err().contains("at most 128"));

    let mut deep = String::from("{ contains = [\"9\"] }");
    for level in (1..=8).rev() {
        deep = format!("{{ contains = [\"{level}\"], all = [{deep}] }}");
    }
    let deep =
        format!("[[rules]]\nid = \"deep\"\nstate = \"idle\"\ncontains = [\"r\"]\nall = [{deep}]\n");
    assert!(compile(&deep).unwrap_err().contains("max gate depth"));

    let matchers = (0..33)
        .map(|index| format!("\"m{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let wide = format!("[[rules]]\nid = \"many\"\nstate = \"idle\"\ncontains = [{matchers}]\n");
    assert!(compile(&wide).unwrap_err().contains("direct matchers"));

    let long = "x".repeat(hide_screen_rules::MAX_MATCHER_CHARS + 1);
    let long = format!("[[rules]]\nid = \"long\"\nstate = \"idle\"\ncontains = [\"{long}\"]\n");
    assert!(compile(&long).unwrap_err().contains("max length"));
}

#[test]
fn rules_for_a_newer_evaluator_are_refused() {
    let rules = "[[rules]]\nid = \"r\"\nstate = \"idle\"\ncontains = [\"ready\"]\n";
    assert!(compile_at(ENGINE_VERSION, rules).is_ok());
    let error = compile_at(ENGINE_VERSION + 1, rules).unwrap_err();
    assert!(error.contains("require engine"), "{error}");
    let top = "[[rules]]\nid = \"top\"\nstate = \"idle\"\nregion = \"top_non_empty_lines(2)\"\ncontains = [\"r\"]\n";
    assert!(
        compile_at(2, top)
            .unwrap_err()
            .contains("top_non_empty_lines")
    );
    assert!(compile_at(3, top).is_ok());
}

#[test]
fn versions_compare_number_by_number() {
    let version = |text: &str| Version::parse(text).unwrap();
    assert!(version("2026.10.11.1") > version("2026.9.30.9"));
    assert!(version("2026.10.11.10") > version("2026.10.11.9"));
    assert_eq!(version("2026.10"), version("2026.10.0"));
    for bad in ["", "1..2", "1.a", "v1"] {
        assert!(Version::parse(bad).is_err(), "{bad}");
    }
}
