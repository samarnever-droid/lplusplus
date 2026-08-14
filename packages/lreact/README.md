# Lreact 2.0 API

Lreact is a native L++ UI bridge with an optional GPU capability boundary.

## Security contract

The browser bridge is deny-by-default. It exposes health, version, Vulkan
probe/capability queries, and bounded relative-file queries. Arbitrary command
execution is intentionally absent. The native IPC server must enforce the same
allowlist, reject absolute or parent-traversal paths, cap request and response
bodies, and validate the `X-Lreact-Protocol: 1` header.

The bridge also applies request timeouts and renders responses with
`textContent`; native responses must never be inserted into the DOM as HTML.

## Vulkan API

The L++ API is lifecycle-oriented:

```text
config := vulkan_default_config()
device := vulkan_probe(config)
caps := vulkan_capabilities_json(device)
frame := vulkan_begin_frame(device, 0)
result := vulkan_end_frame(frame)
```

`VulkanDevice.status` is `0` when no loader/backend is available, `1` when a
device is ready, and `2` while a frame is active. The current freestanding
package reports an unavailable backend rather than fabricating a GPU name.
This keeps application code stable while a platform Vulkan loader is added.

The JavaScript equivalents are `lpp.vulkan.probe()`,
`lpp.vulkan.capabilities()`, and `lpp.vulkan.renderFrame()`.

## Web backend

The browser target is exposed as a named backend rather than an unrestricted
native bridge:

```js
const backend = window.lpp.web;
const health = await backend.invoke("health");
console.log(backend.kind, backend.capabilities);
```

The Web backend is deny-by-default and enforces loopback-only HTTP by default.
Remote deployments must opt in and use HTTPS. Requests have bounded payloads,
timeouts, concurrency, and rate limits; responses must include the matching
request ID and an `ok` status envelope. The bridge never sends cookies and
never exposes shell execution.

The native HTTP service must implement this contract at `POST /api/invoke`:

```json
{"id":"req_123","cmd":"health","args":"{}"}
```

Successful responses are:

```json
{"id":"req_123","status":"ok","result":{"status":"ok"}}
```

The browser bridge is the completed client-side backend boundary. A native
listener is intentionally left for a future ABI-safe implementation: the
current freestanding L++ managed-string boundary still overflows the stack
when a live HTTP request crosses the executable/module boundary. Keep the
native window and browser bridge usable independently until that compiler/runtime
issue is fixed.
