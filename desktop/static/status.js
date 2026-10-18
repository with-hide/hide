// The status page reads its state from its hash (`#connecting`,
// `#failed=<reason>`, with `&file=<path>` when a stored file stopped the
// daemon, and `&said=<sentence>&builds=<line>` when the core is newer than
// this app or its update failed) and asks for Retry by setting `#retry`, and
// for Copy diagnostics by adding `&copy=1`, which the host sees as an in-page
// navigation; the host answers a copy with `&copied=1`, or with
// `&diagnostics=<text>` when the clipboard refused it. It takes
// no part in the window's bridge: the preload is loaded, but the host
// refuses every privileged message from this address (`fromShell`).
// Its sentences arrive in its address, written by the host from the
// interface catalogs in the language in effect; a missing one is a host bug
// and fails loudly rather than showing a guess.

const params = new URLSearchParams(location.search);

function words(key) {
  const value = params.get(key);
  if (value === null) throw new Error("status page was opened without its " + key + " text");
  return value;
}

document.documentElement.lang = words("lang");
// `hidden`: macOS draws its window buttons over the page's top, which stays a drag strip.
document.body.dataset.titlebar = words("titlebar");
document.getElementById("connecting").textContent = words("connecting");
document.getElementById("retry").textContent = words("retry");
const copy = document.getElementById("copy");
const REASONS = ["cli_missing", "start_failed", "no_response", "other_build", "state_refused", "core_newer", "update_failed"];
// The reasons a build names its own title for: the app or the core is the one to update, not a failure to connect.
const TITLES = { core_newer: "core_newer_title", update_failed: "update_failed_title" };

function shownLine(id, text) {
  const line = document.getElementById(id);
  line.textContent = text ?? "";
  line.hidden = !line.textContent;
}

function show() {
  const hash = new URLSearchParams(location.hash.slice(1));
  const failed = hash.get("failed");
  shownLine("file", hash.get("file"));
  shownLine("builds", hash.get("builds"));
  document.getElementById("connecting").hidden = failed !== null;
  document.getElementById("failed").hidden = failed === null;
  copy.textContent = words(hash.has("copied") ? "copied" : "copy");
  const diagnostics = document.getElementById("diagnostics");
  diagnostics.textContent = hash.get("diagnostics") ?? "";
  diagnostics.hidden = !diagnostics.textContent;
  if (failed !== null) {
    const known = REASONS.includes(failed) ? failed : "start_failed";
    document.getElementById("failed-title").textContent = words(TITLES[known] ?? "failed");
    const reason = document.getElementById("reason");
    reason.textContent = hash.get("said") ?? words(TITLES[known] ? "start_failed" : known);
    reason.dataset.reason = failed;
  }
}

document.getElementById("retry").addEventListener("click", () => {
  location.hash = "retry";
});
copy.addEventListener("click", () => {
  const hash = new URLSearchParams(location.hash.slice(1));
  hash.delete("copied");
  hash.delete("diagnostics");
  hash.set("copy", "1");
  location.hash = hash.toString();
});
window.addEventListener("hashchange", show);
show();
