"""The journey table as report.md and report.json, and as the terminal shows it."""

import json

from agent_live_check.protection import write_private

from .cells import CELLS
from .ci import redact


def text(value):
    return str(value).replace("|", "\\|").replace("\n", " ").replace("\r", " ")


def table_lines(report):
    lines = ["| Agent | " + " | ".join(CELLS) + " |", "| --- |" + " --- |" * len(CELLS)]
    for agent, cells in report["table"].items():
        lines.append("| " + agent + " | " + " | ".join(cells[name]["status"] for name in CELLS) + " |")
    return lines


def markdown(report):
    lines = ["# Agent journey", "",
             f"Mode: {report['mode']}. Herdr: {text(report['herdr'].get('version', 'not started'))}. "
             f"Candidate hided sha256: {text(report['herdr'].get('hided_sha256', 'not started'))}.", "",
             *(["**Fixture run**: the compiled stand-ins of web/e2e/shims answered in place of the CLIs and no model "
                "read a prompt; this report is never a release's agent evidence.", ""] if report.get("fixture") else []),
             *(["**Fake model run**: the real CLIs with Hide's real kit, answered by the journey's fake model instead "
                "of a provider; its rows are apart from a signed-in run's and never prove a provider or a login.", ""]
               if report.get("fake_model") else []),
             "Each cell is judged from what Hide reported (spawn answer, snapshot rows, Hide's conversation reader, "
             "the letter record, Herdr's session report), never from screen text.", "",
             *table_lines(report), "",
             "## Every cell", "", "| Agent | Cell | Result | Reason | Evidence |", "| --- | --- | --- | --- | --- |"]
    for agent, cells in report["table"].items():
        for name in CELLS:
            row = cells[name]
            lines.append("| " + " | ".join(text(value) for value in
                                            (agent, name, row["status"], row["reason"], row["evidence"] or "")) + " |")
    if report.get("fixture_limits"):
        lines.extend(["", "## What a stand-in cannot show", ""])
        lines.extend(f"- {text(limit)}" for limit in report["fixture_limits"])
    if report.get("fake_model_limits"):
        lines.extend(["", "## What a fake model cannot show", ""])
        lines.extend(f"- {text(limit)}" for limit in report["fake_model_limits"])
    lines.extend(["", "## Systems", ""])
    lines.extend(f"- {name}: {text(value)}" for name, value in report["systems"].items())
    lines.extend(["", "## What each agent was started with", "",
                  "The spawn arguments are the product's; the route is the only thing local mode adds "
                  "(scripts/agent_journey/local.py).", ""])
    for agent in report["agents"]:
        lines.append(f"### {agent['id']}")
        lines.append("")
        lines.append(f"- CLI: {text(agent.get('version', 'not probed'))}")
        if "requests" in agent:
            budget = agent.get("request_budget")
            lines.append(f"- model requests: {agent['requests']}" + (f" (budget {budget})" if budget is not None else "")
                         + (f", {agent['session_requests']} in its session files" if "session_requests" in agent else ""))
        if agent.get("quota"):
            lines.append(f"- provider quota exhausted: {text(', '.join(agent['quota']))}")
        for key in ("spawn", "start_untrusted"):
            if agent.get(key):
                lines.append(f"- {key} argv: `{text(json.dumps(agent[key]['argv']))}`")
        if agent.get("spawn"):
            lines.append(f"- route: `{text(json.dumps(agent['spawn']['route']))}`")
        lines.append("")
    if report.get("hide_ai_requests"):
        lines.extend(["## Hide AI", "", "The candidate's own model requests, outside every agent's budget: "
                      + text(", ".join(f"{provider} {count}" for provider, count in report["hide_ai_requests"].items())), ""])
    if report.get("product_findings"):
        lines.extend(["## Product findings", ""])
        lines.extend(f"- {text(finding)}" for finding in report["product_findings"])
        lines.append("")
    lines.extend(["## Isolation, operator files and cleanup", "", "```json",
                  json.dumps({key: report[key] for key in ("measured_repository", "measured_commit", "measured_project", "created",
                                               "configuration", "operator_trust", "operator_session_folders",
                                               "leftover_copies", "cleanup", "failures")
                                if key in report},
                             indent=2, default=str), "```", ""])
    return "\n".join(lines)


def save(run, report):
    write_private(run / "report.json", redact((json.dumps(report, indent=2, ensure_ascii=False, default=str) + "\n").encode()))
    write_private(run / "report.md", redact(markdown(report).encode()))
