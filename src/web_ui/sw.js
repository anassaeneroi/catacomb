// Catacomb service worker — deliberately minimal.
//
// Scope: installability + an offline shell, nothing else. It intercepts ONLY
//   * GET navigations to "/" (network-first, cached copy as offline fallback)
//   * the static PWA assets (manifest + icons, cache-first)
// Everything else — /api/*, /ws/*, /files/*, /music-files/*, /feed* — is left
// to the network untouched, so auth, byte-range video streaming and
// WebSockets behave exactly as they do without a service worker.
//
// The shell mirrors the server's no-store policy on "/": every online
// navigation refetches the HTML, and the cache is only consulted when the
// network is unreachable (offline launch of the installed app). Binary
// upgrades therefore reach clients on the next online load, same as before.
const CACHE = 'catacomb-shell-v1';
const STATIC = ['/manifest.webmanifest', '/icons/icon-192.png', '/icons/icon-512.png'];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE).then((c) => c.addAll(STATIC)).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  if (req.mode === 'navigate' && url.pathname === '/') {
    e.respondWith(
      fetch(req)
        .then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put('/', copy));
          }
          return res;
        })
        .catch(() => caches.match('/').then((hit) => hit || Response.error()))
    );
    return;
  }

  if (STATIC.includes(url.pathname)) {
    e.respondWith(caches.match(req).then((hit) => hit || fetch(req)));
  }
});
