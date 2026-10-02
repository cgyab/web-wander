// WebWander service worker — lets the installed PWA launch and play offline.
// Without this, a no-network launch just fails to fetch index.html/the bundle/
// game.wasm and the app won't start. Bump CACHE whenever this file changes to
// roll the cache cleanly.
const CACHE = "webwander-v1";

// Stable (non-fingerprinted) shell files we can name up front. Vite fingerprints
// everything under /assets/ (the name changes each build), so those are cached at
// runtime on first fetch instead of being listed here.
const SHELL = [
  "./",
  "./index.html",
  "./manifest.webmanifest",
  "./game.wasm",
  "./favicon.svg",
  "./icon-192.png",
  "./icon-512.png",
];

self.addEventListener("install", (e) => {
  self.skipWaiting();
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).catch(() => {}));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== location.origin) return; // only ever handle our own assets

  const isNavigation = req.mode === "navigate";
  // The non-fingerprinted shell changes without a new filename, so prefer the
  // network when online (a redeploy is picked up) and fall back to cache offline.
  const isShell =
    isNavigation ||
    url.pathname.endsWith("/game.wasm") ||
    url.pathname.endsWith("/manifest.webmanifest") ||
    url.pathname.endsWith("/index.html") ||
    url.pathname.endsWith("/");

  if (isShell) {
    e.respondWith(
      fetch(req)
        .then((res) => {
          const copy = res.clone();
          // Cache navigations under the canonical index.html so the offline
          // fallback below always finds them.
          caches.open(CACHE).then((c) => c.put(isNavigation ? "./index.html" : req, copy)).catch(() => {});
          return res;
        })
        .catch(() => caches.match(req).then((m) => m || caches.match("./index.html")))
    );
    return;
  }

  // Fingerprinted bundles + icons are immutable: serve from cache, fetching and
  // caching on a miss (this is what populates the cache on the first online run).
  e.respondWith(
    caches.match(req).then(
      (hit) =>
        hit ||
        fetch(req).then((res) => {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
          return res;
        })
    )
  );
});
