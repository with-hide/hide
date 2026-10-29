// The phone app's service worker (PRD mobile-companion D-16, D-21, B23,
// B31, B32, B34). It caches only the static shell: the page, its hashed
// assets, the manifest and the icons, so the app opens without a network and
// shows the unreachable line rather than a blank page. Agent state, rows,
// replies and the push subscription travel on /ws and are never cached here.
//
// A push carries the core's chosen row sentence as detail (or, for a question
// the core asks for the agent, its subject, worded here), the status_code,
// the agent's title, its state
// ("needs_you" | "done" | "observer_unconfirmed" | "letter_undelivered") and
// its place. The words for the state belong to the
// language the phone page is in, so the page posts `{type: "words", words}` on
// load, on every language change and when a subscription is created, and this
// worker keeps them in the Cache API (a closed app must still notify). The
// body joins the row's status word, its untranslated detail and the project
// with " · ", with or without detail (status-marks B22). A payload with no
// status_code, or a cached page that sends only the section words, says the
// section; with no stored words, only detail and place can be drawn.
//
// There is no start_url in the manifest on purpose: the Home Screen app keeps
// the address it was added from, whose fragment carries the phone's
// credential into an installed app whose storage iOS may keep apart.

const CACHE = "hide-phone-shell-v1";
const WORDS_CACHE = "hide-phone-words";
const WORDS_URL = "/m/words.json";
const SHELL = ["/m/", "/m/manifest.webmanifest", "/m/icon-192.png", "/m/icon-512.png", "/m/apple-touch-icon.png"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((key) => key !== CACHE && key !== WORDS_CACHE).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

function isShell(url) {
  // The words store is this worker's own state: neither cached as shell nor
  // answered from the network.
  if (url.pathname === WORDS_URL) return false;
  return url.origin === self.location.origin && (url.pathname.startsWith("/m/") || url.pathname.startsWith("/assets/"));
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET" || !isShell(url) || url.pathname === "/m/sw.js") return;
  if (url.pathname.startsWith("/assets/")) {
    // Hashed file names never change content: the cache answers first.
    event.respondWith(
      caches.match(request).then(
        (hit) =>
          hit ??
          fetch(request).then((response) => {
            if (response.ok) {
              const copy = response.clone();
              void caches.open(CACHE).then((cache) => cache.put(request, copy));
            }
            return response;
          }),
      ),
    );
    return;
  }
  // The page and the manifest: the network first, so a new build lands at once.
  const key = request.mode === "navigate" ? "/m/" : request;
  event.respondWith(
    fetch(request)
      .then((response) => {
        if (response.ok) {
          const copy = response.clone();
          void caches.open(CACHE).then((cache) => cache.put(key, copy));
        }
        return response;
      })
      .catch(() => caches.match(key).then((hit) => hit ?? Response.error())),
  );
});

function isWord(value) {
  return typeof value === "string" && value.length > 0;
}

// Only notification words are kept. Row-status and delivery words are
// optional, so a page from before them still works.
self.addEventListener("message", (event) => {
  const data = event.data;
  if (!data || data.type !== "words" || !data.words || typeof data.words !== "object") return;
  const { needs_you, done, observer_unconfirmed, letter_undelivered } = data.words;
  if (!isWord(needs_you) || !isWord(done)) return;
  const statusWords = Object.fromEntries([...["question", "approval", "error", "working", "done", "idle", "unknown", "waiting", "stopped", "unfinished"].map((code) => `status_${code}`), "subject_plan_review", "subject_mission_decision"].map((key) => [key, data.words[key]]).filter(([, word]) => isWord(word)));
  const words = JSON.stringify({ needs_you, done, observer_unconfirmed, letter_undelivered, ...statusWords });
  event.waitUntil(
    caches
      .open(WORDS_CACHE)
      .then((cache) => cache.put(WORDS_URL, new Response(words, { headers: { "Content-Type": "application/json" } }))),
  );
});

// A question the core asks for the agent comes as its subject; the page
// posted its words with `{{count}}` left for the steps. A mission decision
// carries its own line, led by the word the page posted for it. A cause
// this worker does not know falls back to the payload's plain sentence.
function subjectWords(words, subject) {
  if (!subject || typeof subject !== "object" || typeof subject.cause !== "string") return null;
  if (subject.cause === "mission_decision") return isWord(subject.line) ? subject.line : null;
  const template = words[`subject_${subject.cause}`];
  return isWord(template) ? template.replace("{{count}}", String(subject.steps)) : null;
}

function subjectLead(words, subject) {
  return subject && subject.cause === "mission_decision" && isWord(words.subject_mission_decision) ? words.subject_mission_decision : null;
}

async function storedWords() {
  const hit = await (await caches.open(WORDS_CACHE)).match(WORDS_URL);
  if (!hit) return {};
  try {
    const words = await hit.json();
    return words && typeof words === "object" ? words : {};
  } catch {
    return {};
  }
}

async function closeTags(tags) {
  if (!Array.isArray(tags) || tags.length === 0) return;
  const shown = await self.registration.getNotifications();
  for (const notification of shown) if (tags.includes(notification.tag)) notification.close();
}

self.addEventListener("push", (event) => {
  let payload = null;
  try {
    payload = event.data ? event.data.json() : null;
  } catch {
    payload = null;
  }
  if (!payload || typeof payload.title !== "string") return;
  event.waitUntil(
    Promise.all([closeTags(payload.clear), storedWords()]).then(([, words]) =>
      self.registration.showNotification(payload.title, {
        body: [subjectLead(words, payload.subject) || words[`status_${payload.status_code}`] || words[payload.state], subjectWords(words, payload.subject) || payload.detail, payload.place].filter(isWord).join(" · "),
        tag: payload.tag,
        icon: "/m/icon-192.png",
        badge: "/m/icon-192.png",
        data: { device_id: payload.device_id, pane_id: payload.pane_id },
      }),
    ),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data ?? {};
  const tag = data.device_id && data.pane_id ? `${data.device_id}|${data.pane_id}` : null;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      const open = windows.find((client) => new URL(client.url).pathname.startsWith("/m/"));
      if (open) {
        if (tag) open.postMessage({ type: "open", tag });
        return open.focus();
      }
      return self.clients.openWindow(tag ? `/m/#open=${encodeURIComponent(tag)}` : "/m/");
    }),
  );
});
