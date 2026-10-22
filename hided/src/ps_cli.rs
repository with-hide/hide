//! `hide ps`: the live panes and agents as one tree, project > checkout >
//! tab > pane, with each delegated agent under the agent that delegated it.
//!
//! The core answers flat rows (`herdr_core::delivery::answer::Ps`); the tree
//! is drawn here. Every pane is printed exactly once: a delegated agent's
//! pane is printed under its parent, with where it lives when that is not
//! the parent's tab, and not again in its own tab.
use crate::env::Env;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub const USAGE: &str = "hide ps [--all] [--here] [--json]";

const UNREADABLE: &str =
    "Run the hide that came with the running Hide, whose answer this build can read";

#[derive(Debug, Eq, PartialEq)]
pub struct Options {
    /// Ended registrations too.
    pub all: bool,
    /// Only the project of the pane the command runs in.
    pub here: bool,
    pub json: bool,
}

pub fn parse<'a>(args: impl Iterator<Item = &'a String>) -> Result<Options, String> {
    let mut options = Options {
        all: false,
        here: false,
        json: false,
    };
    for arg in args {
        let flag = match arg.as_str() {
            "--all" => &mut options.all,
            "--here" => &mut options.here,
            "--json" => &mut options.json,
            _ => return Err(USAGE.to_owned()),
        };
        if *flag {
            return Err(USAGE.to_owned());
        }
        *flag = true;
    }
    Ok(options)
}

pub fn run(env: &Env, options: &Options) -> Result<(), String> {
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_ps(&mut credential, options.all)
    })();
    let refusal = |code: String, message: &str| {
        println!(
            "{}",
            json!({"ok":false,"error":{"code":code,"message":message}})
        );
        Err(code)
    };
    let value = match answer {
        Ok(answer) if answer["ok"] == true => answer["result"].clone(),
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("ps_unavailable")
                .to_owned();
            let message = answer["next_action"].as_str().unwrap_or(&code).to_owned();
            return refusal(code, &message);
        }
        Err(code) => {
            let message = code.clone();
            return refusal(code, &message);
        }
    };
    let value = match select(value, options.here) {
        Ok(value) => value,
        Err("here_unavailable") => {
            return refusal(
                "here_unavailable".to_owned(),
                "Run hide ps --here inside a pane of a project Hide shows, or leave out --here",
            );
        }
        Err(code) => return refusal(code.to_owned(), UNREADABLE),
    };
    if options.json {
        println!("{}", json!({"ok":true,"value":value}));
    } else {
        match read(&value) {
            Ok(answer) => print!("{}", draw(&answer)),
            Err(code) => return refusal(code.to_owned(), UNREADABLE),
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct Pane {
    machine: String,
    machine_label: Option<String>,
    project: String,
    project_path: String,
    checkout: String,
    checkout_path: String,
    worktree: bool,
    tab: Option<String>,
    tab_id: Option<String>,
    pane: String,
    title: Option<String>,
    you: bool,
    agent: Option<Agent>,
}

#[derive(Deserialize)]
struct Agent {
    id: Option<String>,
    name: Option<String>,
    kind: String,
    state: String,
    label: Option<String>,
    machine: String,
    pane: String,
    parent_machine: Option<String>,
    parent_pane: Option<String>,
    watched_by: Option<String>,
    mission: Option<Mission>,
    project: Option<String>,
}

#[derive(Deserialize)]
struct Mission {
    title: String,
    role: String,
}

#[derive(Deserialize)]
struct Answer {
    panes: Vec<Pane>,
    agents: Vec<Agent>,
    missions_unreadable: bool,
}

/// The answer as this build reads it; a daemon of another build may answer
/// another shape, which is refused rather than drawn as nothing.
fn read(value: &Value) -> Result<Answer, &'static str> {
    serde_json::from_value(value.clone()).map_err(|_| "ps_answer_unreadable")
}

/// Which panes print at the top of their tab, and which under a parent: a
/// pane whose agent names a live parent pane is the parent's child, unless
/// following parents comes back to it.
struct Lineage {
    children: HashMap<usize, Vec<usize>>,
    roots: Vec<usize>,
}

fn lineage(panes: &[Pane]) -> Lineage {
    let at: HashMap<(&str, &str), usize> = panes
        .iter()
        .enumerate()
        .map(|(index, pane)| ((pane.machine.as_str(), pane.pane.as_str()), index))
        .collect();
    let parent = |index: usize| {
        let agent = panes[index].agent.as_ref()?;
        let key = (
            agent.parent_machine.as_deref()?,
            agent.parent_pane.as_deref()?,
        );
        at.get(&key).copied().filter(|&parent| parent != index)
    };
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for index in 0..panes.len() {
        // A pane whose parents lead back to it would be under nobody that
        // prints; one that only hangs off such a loop is under its parent.
        let mut seen = HashSet::from([index]);
        let mut up = parent(index);
        let mut cycle = false;
        while let Some(above) = up {
            if above == index {
                cycle = true;
                break;
            }
            if !seen.insert(above) {
                break;
            }
            up = parent(above);
        }
        match parent(index).filter(|_| !cycle) {
            Some(above) => children.entry(above).or_default().push(index),
            None => roots.push(index),
        }
    }
    Lineage { children, roots }
}

/// The answer `hide ps` prints: all of it, or with `here` only the caller's
/// project, its panes' delegated agents wherever they live, and the
/// registrations that name one of its checkouts.
fn select(value: Value, here: bool) -> Result<Value, &'static str> {
    if !here {
        return Ok(value);
    }
    let answer = read(&value)?;
    let you = answer
        .panes
        .iter()
        .find(|pane| pane.you)
        .ok_or("here_unavailable")?;
    let project = (you.machine.as_str(), you.project_path.as_str());
    let lineage = lineage(&answer.panes);
    let mut kept: HashSet<usize> = HashSet::new();
    // The project's panes, a child of another project's agent included,
    // and their delegated agents wherever those live.
    let mut stack: Vec<usize> = (0..answer.panes.len())
        .filter(|&index| {
            let pane = &answer.panes[index];
            (pane.machine.as_str(), pane.project_path.as_str()) == project
        })
        .collect();
    while let Some(index) = stack.pop() {
        if kept.insert(index) {
            stack.extend(lineage.children.get(&index).into_iter().flatten());
        }
    }
    let paths: HashSet<&str> = answer
        .panes
        .iter()
        .filter(|pane| (pane.machine.as_str(), pane.project_path.as_str()) == project)
        .map(|pane| pane.checkout_path.as_str())
        .chain([project.1])
        .collect();
    let mut value = value;
    let panes = value["panes"].as_array_mut().ok_or("ps_unavailable")?;
    let mut index = 0;
    panes.retain(|_| {
        index += 1;
        kept.contains(&(index - 1))
    });
    let agents = value["agents"].as_array_mut().ok_or("ps_unavailable")?;
    let mut index = 0;
    agents.retain(|_| {
        index += 1;
        let agent = &answer.agents[index - 1];
        agent.machine == project.0
            && agent
                .project
                .as_deref()
                .is_some_and(|path| paths.contains(path))
    });
    Ok(value)
}

/// A line of the tree and the lines under it.
struct Node {
    text: String,
    children: Vec<Node>,
}

fn node(text: String) -> Node {
    Node {
        text,
        children: Vec::new(),
    }
}

fn agent_text(agent: &Agent) -> String {
    let mut text = match (&agent.name, &agent.id) {
        (Some(name), Some(id)) if name != id => format!("{name} [{id}]"),
        (Some(name), _) => name.clone(),
        (None, Some(id)) => id.clone(),
        (None, None) => "-".to_owned(),
    };
    text.push_str(&format!("  {}  {}", agent.kind, agent.state));
    if let Some(label) = &agent.label {
        text.push_str(&format!("  \"{label}\""));
    }
    if let Some(mission) = &agent.mission {
        text.push_str(&format!("  mission \"{}\" {}", mission.title, mission.role));
    }
    if let Some(observer) = &agent.watched_by {
        text.push_str(&format!("  (watched by {observer})"));
    }
    text
}

fn pane_text(pane: &Pane, under: Option<&Pane>) -> String {
    let mut text = format!("{}  ", pane.pane);
    match &pane.agent {
        Some(agent) => text.push_str(&agent_text(agent)),
        None => {
            text.push_str("terminal");
            if let Some(title) = &pane.title {
                text.push_str(&format!("  \"{title}\""));
            }
        }
    }
    if let Some(parent) = under {
        let mut place = Vec::new();
        if parent.machine != pane.machine {
            place.push(format!(
                "on {}",
                pane.machine_label.as_deref().unwrap_or(&pane.machine)
            ));
        }
        if (parent.machine.as_str(), parent.project_path.as_str())
            != (pane.machine.as_str(), pane.project_path.as_str())
        {
            place.push(pane.project.clone());
        }
        if parent.checkout_path != pane.checkout_path || !place.is_empty() {
            place.push(checkout_text(pane));
        }
        if parent.tab_id != pane.tab_id || !place.is_empty() {
            place.push(tab_text(pane));
        }
        if !place.is_empty() {
            text.push_str(&format!("  @ {}", place.join(" · ")));
        }
    }
    if pane.you {
        text.push_str("  <- you");
    }
    text
}

fn checkout_text(pane: &Pane) -> String {
    if pane.worktree {
        format!("worktree {}", pane.checkout)
    } else {
        format!("checkout {}", pane.checkout)
    }
}

fn tab_text(pane: &Pane) -> String {
    format!(
        "tab {}",
        pane.tab
            .as_deref()
            .or(pane.tab_id.as_deref())
            .unwrap_or("-")
    )
}

fn pane_node(panes: &[Pane], lineage: &Lineage, index: usize, under: Option<usize>) -> Node {
    let mut line = node(pane_text(&panes[index], under.map(|parent| &panes[parent])));
    for &child in lineage.children.get(&index).into_iter().flatten() {
        line.children
            .push(pane_node(panes, lineage, child, Some(index)));
    }
    line
}

/// The text `hide ps` prints for `answer`.
fn draw(answer: &Answer) -> String {
    let panes = &answer.panes;
    let lineage = lineage(panes);
    // Projects, checkouts and tabs in the order the core lists their panes.
    let mut projects: Vec<Node> = Vec::new();
    let mut at: HashMap<(&str, &str), usize> = HashMap::new();
    let mut checkouts: HashMap<(usize, &str), usize> = HashMap::new();
    let mut tabs: HashMap<(usize, usize, Option<&str>), usize> = HashMap::new();
    for &index in &lineage.roots {
        let pane = &panes[index];
        let project = *at
            .entry((pane.machine.as_str(), pane.project_path.as_str()))
            .or_insert_with(|| {
                let mut text = format!("{}  {}", pane.project, pane.project_path);
                if let Some(machine) = &pane.machine_label {
                    text.push_str(&format!("  @ {machine}"));
                }
                projects.push(node(text));
                projects.len() - 1
            });
        let checkout = *checkouts
            .entry((project, pane.checkout_path.as_str()))
            .or_insert_with(|| {
                projects[project].children.push(node(checkout_text(pane)));
                projects[project].children.len() - 1
            });
        let tab = *tabs
            .entry((project, checkout, pane.tab_id.as_deref()))
            .or_insert_with(|| {
                let line = &mut projects[project].children[checkout];
                line.children.push(node(tab_text(pane)));
                line.children.len() - 1
            });
        projects[project].children[checkout].children[tab]
            .children
            .push(pane_node(panes, &lineage, index, None));
    }
    let mut out = String::new();
    for project in &projects {
        out.push_str(&project.text);
        out.push('\n');
        lines(&project.children, "", &mut out);
    }
    for (title, ended) in [("not in a pane", false), ("ended", true)] {
        let rows: Vec<Node> = answer
            .agents
            .iter()
            .filter(|agent| (agent.state == "ended") == ended)
            .map(|agent| {
                let mut text = format!("{}  {}", agent.pane, agent_text(agent));
                if let Some(project) = &agent.project {
                    text.push_str(&format!("  {project}"));
                }
                node(text)
            })
            .collect();
        if !rows.is_empty() {
            out.push_str(title);
            out.push('\n');
            lines(&rows, "", &mut out);
        }
    }
    if out.is_empty() {
        out.push_str("no live panes\n");
    }
    if answer.missions_unreadable {
        out.push_str(
            "missions: the mission record could not be read, so no row names its mission\n",
        );
    }
    out
}

/// Box-drawing glyphs: Rust writes UTF-8 to a pipe and UTF-16 to a Windows
/// console, so every terminal Hide supports draws them.
fn lines(nodes: &[Node], prefix: &str, out: &mut String) {
    for (index, line) in nodes.iter().enumerate() {
        let last = index + 1 == nodes.len();
        out.push_str(prefix);
        out.push_str(if last { "└─ " } else { "├─ " });
        out.push_str(&line.text);
        out.push('\n');
        let deeper = format!("{prefix}{}", if last { "   " } else { "│  " });
        lines(&line.children, &deeper, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: &str = "mac";

    fn pane(project: &str, checkout: &str, tab: &str, id: &str, agent: Option<Value>) -> Value {
        json!({
            "machine": HERE, "machine_label": null,
            "project": project, "project_path": format!("/p/{project}"),
            "checkout": checkout, "checkout_path": format!("/p/{project}/{checkout}"),
            "worktree": checkout != "main",
            "tab": tab, "tab_id": format!("t-{tab}"),
            "pane": id, "title": agent.is_none().then_some("zsh"), "you": false,
            "agent": agent,
        })
    }

    fn agent(id: &str, name: &str, pane: &str, parent: Option<&str>) -> Value {
        json!({
            "id": id, "name": name, "kind": "claude", "state": "going",
            "label": format!("{name} task"), "line": null,
            "machine": HERE, "pane": pane,
            "parent_machine": parent.map(|_| HERE), "parent_pane": parent,
            "parent": null, "watched_by": parent.map(|_| "lead"),
            "mission": null, "project": "/p/hide/main",
        })
    }

    fn fixture() -> Value {
        json!({
            "panes": [
                pane("hide", "main", "Agents", "w1:p1", Some(agent("agent-1", "lead", "w1:p1", None))),
                pane("hide", "main", "Agents", "w1:p2", None),
                pane("hide", "feat/x", "x", "w2:p1", Some(agent("agent-2", "child", "w2:p1", Some("w1:p1")))),
                pane("site", "main", "web", "w3:p1", Some(agent("agent-3", "grandchild", "w3:p1", Some("w2:p1")))),
                pane("site", "main", "web", "w3:p2", None),
            ],
            "agents": [],
            "missions_unreadable": false,
        })
    }

    #[test]
    fn every_pane_prints_once_with_children_under_their_parent_and_where_they_live() {
        let text = draw(&read(&fixture()).unwrap());
        assert_eq!(
            text,
            "hide  /p/hide\n\
             └─ checkout main\n\
             \x20  └─ tab Agents\n\
             \x20     ├─ w1:p1  lead [agent-1]  claude  going  \"lead task\"\n\
             \x20     │  └─ w2:p1  child [agent-2]  claude  going  \"child task\"  (watched by lead)  @ worktree feat/x · tab x\n\
             \x20     │     └─ w3:p1  grandchild [agent-3]  claude  going  \"grandchild task\"  (watched by lead)  @ site · checkout main · tab web\n\
             \x20     └─ w1:p2  terminal  \"zsh\"\n\
             site  /p/site\n\
             └─ checkout main\n\
             \x20  └─ tab web\n\
             \x20     └─ w3:p2  terminal  \"zsh\"\n"
        );
        for id in ["w1:p1", "w1:p2", "w2:p1", "w3:p1", "w3:p2"] {
            assert_eq!(text.matches(id).count(), 1, "{id}");
        }
    }

    #[test]
    fn a_parent_cycle_prints_its_panes_as_roots_instead_of_losing_them() {
        let mut value = fixture();
        value["panes"][0]["agent"]["parent_machine"] = json!(HERE);
        value["panes"][0]["agent"]["parent_pane"] = json!("w3:p1");
        // An agent hanging off the loop stays under its parent.
        value["panes"].as_array_mut().unwrap().push(pane(
            "hide",
            "main",
            "Agents",
            "w4:p1",
            Some(agent("agent-4", "tail", "w4:p1", Some("w1:p1"))),
        ));
        let answer = read(&value).unwrap();
        let text = draw(&answer);
        for id in ["w1:p1", "w2:p1", "w3:p1", "w4:p1"] {
            assert_eq!(text.matches(id).count(), 1, "{id}\n{text}");
        }
        let lineage = lineage(&answer.panes);
        assert_eq!(lineage.roots, [0, 1, 2, 3, 4]);
        assert_eq!(lineage.children.get(&0), Some(&vec![5]));
    }

    #[test]
    fn here_keeps_the_callers_project_and_its_delegated_agents_elsewhere() {
        let mut value = fixture();
        value["panes"][1]["you"] = json!(true);
        let mut other = agent("agent-8", "other", "w8:p8", None);
        other["project"] = json!("/q/other");
        value["agents"] = json!([agent("agent-9", "ended", "w9:p9", None), other]);
        value["agents"][0]["state"] = json!("ended");
        let kept = select(value, true).unwrap();
        let panes: Vec<&str> = kept["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| pane["pane"].as_str().unwrap())
            .collect();
        assert_eq!(panes, ["w1:p1", "w1:p2", "w2:p1", "w3:p1"]);
        let agents: Vec<&str> = kept["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|agent| agent["id"].as_str().unwrap())
            .collect();
        assert_eq!(agents, ["agent-9"]);
        assert_eq!(select(fixture(), true), Err("here_unavailable"));
    }

    /// A pane of the project whose parent lives in another project is kept,
    /// the caller's own pane among them, and is drawn in its own tab.
    #[test]
    fn here_keeps_a_pane_of_the_project_whose_parent_lives_elsewhere() {
        let mut value = fixture();
        value["panes"][3]["you"] = json!(true);
        let kept = select(value, true).unwrap();
        let panes: Vec<&str> = kept["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pane| pane["pane"].as_str().unwrap())
            .collect();
        assert_eq!(panes, ["w3:p1", "w3:p2"]);
        let text = draw(&read(&kept).unwrap());
        assert!(
            text.contains("tab web\n      ├─ w3:p1  grandchild"),
            "{text}"
        );
    }

    #[test]
    fn agents_without_a_pane_and_ended_ones_print_in_their_own_sections() {
        let mut value = json!({"panes": [], "missions_unreadable": true, "agents": [
            agent("agent-4", "asleep", "w4:p1", None),
            agent("agent-5", "gone", "w5:p1", None),
        ]});
        value["agents"][0]["state"] = json!("asleep");
        value["agents"][1]["state"] = json!("ended");
        assert_eq!(
            draw(&read(&value).unwrap()),
            "not in a pane\n\
             └─ w4:p1  asleep [agent-4]  claude  asleep  \"asleep task\"  /p/hide/main\n\
             ended\n\
             └─ w5:p1  gone [agent-5]  claude  ended  \"gone task\"  /p/hide/main\n\
             missions: the mission record could not be read, so no row names its mission\n"
        );
        assert_eq!(
            draw(&read(&json!({"panes": [], "agents": [], "missions_unreadable": false})).unwrap()),
            "no live panes\n"
        );
        assert!(read(&json!({"panes": [{}], "agents": [], "missions_unreadable": false})).is_err());
    }

    #[test]
    fn options_are_switches_given_at_most_once() {
        let line = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            parse(args.iter())
        };
        assert_eq!(
            line(&["--all", "--here", "--json"]).unwrap(),
            Options {
                all: true,
                here: true,
                json: true
            }
        );
        assert!(line(&["--all", "--all"]).is_err());
        assert!(line(&["here"]).is_err());
    }
}
