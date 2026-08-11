/**
 * lreact.js — Lreact Client Bridge SDK for React / Vue / Svelte / Vanilla JS
 * Connects web frontends to native L++ backends & Vulkan GPU over zero-dependency IPC.
 */

(function (window) {
    const LREACT_PORT = 8765;
    const SERVER_URL = `http://127.0.0.1:${LREACT_PORT}`;

    const listeners = new Map();

    /**
     * Invokes a native L++ command handler from JavaScript.
     * @param {string} cmd - Command name registered in L++ backend
     * @param {object} args - Parameters object passed to L++
     * @returns {Promise<any>}
     */
    async function invoke(cmd, args = {}) {
        const reqId = "req_" + Math.random().toString(36).substr(2, 9);
        try {
            const response = await fetch(`${SERVER_URL}/api/invoke`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    id: reqId,
                    cmd: cmd,
                    args: JSON.stringify(args)
                })
            });
            const data = await response.json();
            return data.result;
        } catch (err) {
            console.error(`[Lreact IPC Error] Failed to invoke command '${cmd}':`, err);
            throw err;
        }
    }

    /**
     * Listens for real-time events emitted by the L++ native backend.
     * @param {string} eventName
     * @param {function} callback
     */
    function listen(eventName, callback) {
        if (!listeners.has(eventName)) {
            listeners.set(eventName, []);
        }
        listeners.get(eventName).push(callback);

        return () => {
            const list = listeners.get(eventName) || [];
            listeners.set(eventName, list.filter(cb => cb !== callback));
        };
    }

    // Expose window.lpp API namespace (Tauri/Electron + Vulkan GPU style)
    window.lpp = {
        invoke,
        listen,
        vulkan: {
            getGPUInfo: () => invoke("get_vulkan_info", {}),
            renderFrame: () => invoke("vulkan_render_frame", {}),
        },
        fs: {
            readFile: (path) => invoke("read_file", { path }),
            writeFile: (path, content) => invoke("write_file", { path, content }),
            exists: (path) => invoke("file_exists", { path }),
        },
        shell: {
            exec: (command) => invoke("shell_exec", { command }),
        },
        app: {
            getVersion: () => invoke("get_version", {}),
            quit: () => invoke("quit_app", {}),
        }
    };

    console.log("[Lreact] Bridge SDK initialized with Vulkan GPU support.");
})(window);
