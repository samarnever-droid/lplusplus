const CACHE_NAME = "samarbook-offline-cache-v2";
const STATIC_URLS = [
  "/",
  "/app",
  "/graph",
  "/variables",
  "/assets",
  "/plugins",
  "/documentations",
  "/manifest.webmanifest",
  "/favicon.svg",
];

// On install, pre-cache core pages
self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => {
        return cache.addAll(STATIC_URLS);
      })
      .then(() => self.skipWaiting()),
  );
});

// On activation, clean up old caches
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => {
        return Promise.all(
          keys.map((key) => {
            if (key !== CACHE_NAME) {
              return caches.delete(key);
            }
          }),
        );
      })
      .then(() => self.clients.claim()),
  );
});

// Intercept fetch requests
self.addEventListener("fetch", (event) => {
  // Only handle GET requests and same-origin / static assets
  if (event.request.method !== "GET") return;

  const url = new URL(event.request.url);

  // Skip external APIs (like Wikipedia, Gemini, OpenAI) so they don't get cached
  if (url.origin !== self.location.origin) return;

  // SPA navigation fallback: if navigating to a page and offline, serve the cached app shell
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request)
        .then((networkResponse) => {
          if (networkResponse.status === 200) {
            caches.open(CACHE_NAME).then((cache) => {
              cache.put(event.request, networkResponse.clone());
              cache.put("/", networkResponse.clone());
            });
          }
          return networkResponse;
        })
        .catch(async () => {
          const cachedResponse =
            (await caches.match(event.request)) ||
            (await caches.match(url.pathname)) ||
            (await caches.match("/")) ||
            (await caches.match("/app"));

          if (cachedResponse) {
            return cachedResponse;
          }

          return new Response("Offline", {
            status: 503,
            statusText: "Offline",
            headers: { "Content-Type": "text/plain; charset=utf-8" },
          });
        }),
    );
    return;
  }

  event.respondWith(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.match(event.request).then((cachedResponse) => {
        const fetchPromise = fetch(event.request)
          .then((networkResponse) => {
            if (networkResponse.status === 200) {
              cache.put(event.request, networkResponse.clone());
            }
            return networkResponse;
          })
          .catch((err) => {
            // If network fails and there is no cache, return a fallback or rethrow
            if (cachedResponse) return cachedResponse;
            throw err;
          });

        // Stale-While-Revalidate: return cache instantly if found, else wait for network
        return cachedResponse || fetchPromise;
      });
    }),
  );
});

// Message listener to receive and pre-cache dynamically loaded JS/CSS assets from the main page
self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "PRE_CACHE") {
    const urlsToCache = event.data.urls || [];
    event.waitUntil(
      caches.open(CACHE_NAME).then((cache) => {
        // Filter out URLs we already have or invalid URLs
        const uniqueUrls = [...new Set([...STATIC_URLS, ...urlsToCache])];
        return Promise.all(
          uniqueUrls.map((url) => {
            return fetch(url)
              .then((res) => {
                if (res.status === 200) {
                  return cache.put(url, res.clone());
                }
              })
              .catch((err) => console.warn(`Failed to pre-cache ${url}:`, err));
          }),
        ).then(() => {
          // Notify client that pre-caching is complete
          if (event.source) {
            event.source.postMessage({ type: "PRE_CACHE_COMPLETE" });
          }
        });
      }),
    );
  }
});
