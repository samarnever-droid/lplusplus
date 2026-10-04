/**
 * lreact.js — capability-limited client bridge for Lreact applications.
 *
 * The native endpoint must still enforce the same policy server-side. This
 * client layer prevents accidental exposure from normal UI code and gives
 * callers bounded, cancellable requests.
 */
(function (root) {
    "use strict";

    const config = root.__LREACT_CONFIG__ || {};
    const port = Number.isInteger(config.port) ? config.port : 8765;
    const rawServerUrl = typeof config.serverUrl === "string"
        ? config.serverUrl
        : `http://127.0.0.1:${port}`;
    const timeoutMs = Number.isInteger(config.timeoutMs) ? config.timeoutMs : 5000;
    const maxPayloadBytes = Number.isInteger(config.maxPayloadBytes)
        ? config.maxPayloadBytes
        : 1024 * 1024;
    const maxConcurrent = Number.isInteger(config.maxConcurrent)
        ? Math.max(1, Math.min(config.maxConcurrent, 32))
        : 8;
    const rateLimit = Number.isInteger(config.rateLimit) ? Math.max(1, config.rateLimit) : 60;
    const rateWindowMs = Number.isInteger(config.rateWindowMs) ? Math.max(1000, config.rateWindowMs) : 10000;

    function resolveServerUrl(value) {
        let parsed;
        try {
            parsed = new URL(value, root.location && root.location.href);
        } catch {
            throw new Error("Lreact server URL is invalid");
        }
        const host = parsed.hostname.toLowerCase();
        const loopback = host === "127.0.0.1" || host === "localhost" || host === "::1";
        if (!loopback && config.allowRemote !== true) {
            throw new Error("Lreact Web backend only permits loopback servers by default");
        }
        if (!loopback && parsed.protocol !== "https:") {
            throw new Error("Remote Lreact Web backends require HTTPS");
        }
        if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
            throw new Error("Lreact Web backend requires HTTP or HTTPS");
        }
        return parsed.toString().replace(/\/$/, "");
    }

    const serverUrl = resolveServerUrl(rawServerUrl);

    // Commands are capabilities, not arbitrary strings from the UI.
    const commandSpecs = Object.freeze({
        health: Object.freeze({ maxArgsBytes: 2 }),
        get_version: Object.freeze({ maxArgsBytes: 2 }),
        vulkan_probe: Object.freeze({ maxArgsBytes: 2 }),
        vulkan_capabilities: Object.freeze({ maxArgsBytes: 2 }),
        vulkan_render_frame: Object.freeze({ maxArgsBytes: 128 }),
        read_file: Object.freeze({ maxArgsBytes: 4200, path: true }),
        file_exists: Object.freeze({ maxArgsBytes: 4200, path: true })
    });
    const listeners = new Map();
    let activeRequests = 0;
    let windowStarted = Date.now();
    let windowRequests = 0;

    function requireCommand(cmd) {
        if (typeof cmd !== "string" || !/^[a-z][a-z0-9_.-]{0,63}$/.test(cmd)) {
            throw new TypeError("Lreact command must be a safe identifier");
        }
        if (!Object.prototype.hasOwnProperty.call(commandSpecs, cmd)) {
            throw new Error(`Lreact command is not enabled: ${cmd}`);
        }
        return commandSpecs[cmd];
    }

    function requireRelativePath(path) {
        if (typeof path !== "string" || path.length === 0 || path.length > 4096) {
            throw new TypeError("Lreact paths must be non-empty strings under 4096 bytes");
        }
        if (path.includes("\0") || /^[a-zA-Z]:[\\/]/.test(path) || /^[\\/]/.test(path)) {
            throw new Error("Lreact paths must be relative");
        }
        const parts = path.replaceAll("\\", "/").split("/");
        if (parts.some((part) => part === "..")) {
            throw new Error("Lreact paths may not escape the app data directory");
        }
        return path;
    }

    function requireArgs(args, spec) {
        if (args === null || typeof args !== "object" || Array.isArray(args)) {
            throw new TypeError("Lreact command arguments must be an object");
        }
        const encoded = JSON.stringify(args);
        const encodedBytes = root.TextEncoder
            ? new root.TextEncoder().encode(encoded).length
            : encoded.length;
        if (encodedBytes > maxPayloadBytes || encodedBytes > spec.maxArgsBytes) {
            throw new Error("Lreact request payload is too large");
        }
        return encoded;
    }

    function consumeBudget() {
        const now = Date.now();
        if (now - windowStarted >= rateWindowMs) {
            windowStarted = now;
            windowRequests = 0;
        }
        if (windowRequests >= rateLimit) {
            throw new Error("Lreact Web backend rate limit exceeded");
        }
        if (activeRequests >= maxConcurrent) {
            throw new Error("Lreact Web backend concurrency limit exceeded");
        }
        windowRequests += 1;
        activeRequests += 1;
    }

    async function invoke(cmd, args = {}) {
        const spec = requireCommand(cmd);
        if (spec.path) {
            if (args === null || typeof args !== "object" || typeof args.path !== "string") {
                throw new TypeError("Lreact file commands require a relative path");
            }
            requireRelativePath(args.path);
        }
        const encodedArgs = requireArgs(args, spec);
        consumeBudget();
        const requestId = `req_${root.crypto && root.crypto.randomUUID
            ? root.crypto.randomUUID()
            : Date.now().toString(36)}`;
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), timeoutMs);

        try {
            const response = await fetch(`${serverUrl}/api/invoke`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    "Accept": "application/json",
                    "X-Lreact-Protocol": "1"
                },
                body: JSON.stringify({ id: requestId, cmd, args: encodedArgs }),
                signal: controller.signal,
                credentials: "omit",
                cache: "no-store",
                referrerPolicy: "no-referrer"
            });
            const body = await response.text();
            if (body.length > maxPayloadBytes) {
                throw new Error("Lreact response payload is too large");
            }
            if (!response.ok) {
                throw new Error(`Lreact IPC HTTP ${response.status}`);
            }
            let data;
            try {
                data = JSON.parse(body);
            } catch {
                throw new Error("Lreact IPC returned invalid JSON");
            }
            if (!data || typeof data !== "object" || data.id !== requestId) {
                throw new Error("Lreact IPC response identity mismatch");
            }
            if (data.status === "error" || data.error) throw new Error("Lreact IPC command rejected");
            if (data.status !== "ok" && !Object.prototype.hasOwnProperty.call(data, "result")) {
                throw new Error("Lreact IPC returned an invalid response envelope");
            }
            return data && Object.prototype.hasOwnProperty.call(data, "result")
                ? data.result
                : data;
        } finally {
            clearTimeout(timer);
            activeRequests -= 1;
        }
    }

    function listen(eventName, callback) {
        if (typeof eventName !== "string" || !/^[a-z][a-z0-9_.-]{0,63}$/.test(eventName)) {
            throw new TypeError("Lreact event names must be safe identifiers");
        }
        if (typeof callback !== "function") throw new TypeError("Lreact listener must be a function");
        if (!listeners.has(eventName)) listeners.set(eventName, new Set());
        const callbacks = listeners.get(eventName);
        callbacks.add(callback);
        return () => callbacks.delete(callback);
    }

    function emit(eventName, payload) {
        const callbacks = listeners.get(eventName);
        if (!callbacks) return;
        for (const callback of callbacks) callback(payload);
    }

    function createWebBackend() {
        return Object.freeze({
            name: "lreact-web",
            kind: "web",
            protocol: 1,
            transport: "http-json",
            capabilities: Object.freeze([
                "health",
                "get_version",
                "vulkan_probe",
                "vulkan_capabilities",
                "vulkan_render_frame",
                "read_file",
                "file_exists"
            ]),
            invoke,
            listen,
            emit
        });
    }

    root.lpp = Object.freeze({
        invoke,
        web: createWebBackend(),
        createWebBackend,
        listen,
        emit,
        vulkan: Object.freeze({
            probe: () => invoke("vulkan_probe"),
            capabilities: () => invoke("vulkan_capabilities"),
            renderFrame: () => invoke("vulkan_render_frame")
        }),
        fs: Object.freeze({
            readFile: (path) => invoke("read_file", { path: requireRelativePath(path) }),
            exists: (path) => invoke("file_exists", { path: requireRelativePath(path) })
        }),
        app: Object.freeze({
            health: () => invoke("health"),
            getVersion: () => invoke("get_version")
        })
    });
})(typeof globalThis === "object" ? globalThis : window);
