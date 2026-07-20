(function () {
  const id = "__ID__";
  const host = document.getElementById("term-" + id);
  if (!host || host.dataset.mounted) { return; }
  host.dataset.mounted = "1";
  const base = "http://127.0.0.1:__PORT__/terminal/assets/";
  import(base + "src/index.js")
    .then(async function (mod) {
      const Ferroterm = mod.Ferroterm || mod.default;
      const options = {
        scrollback: 5000,
        fontSize: 13,
        autoFit: true,
        wasmUrl: base + "pkg/ferroterm_wasm_bg.wasm",
      };
      let term;
      try {
        term = await Ferroterm.create(host, { renderer: "webgl", ...options });
      } catch (_e) {
        // WebGL unavailable in this WebView2 session: fall back to Canvas2D.
        term = await Ferroterm.create(host, { renderer: "canvas", ...options });
      }
      let ws = null;
      let closed = false;
      function sendResize() {
        if (ws && ws.readyState === 1) {
          ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }));
        }
      }
      function connect() {
        if (closed || !document.getElementById("term-" + id)) { return; }
        ws = new WebSocket("ws://127.0.0.1:__PORT__/terminal/" + id + "/ws?token=__TOKEN__");
        ws.binaryType = "arraybuffer";
        ws.onopen = function () {
          // The server replays scrollback into this same live terminal on
          // every (re)connect; clear first so replayed bytes don't stack on
          // top of what's already on screen.
          term.clear();
          sendResize();
        };
        ws.onmessage = function (event) {
          if (typeof event.data !== "string") { term.write(new Uint8Array(event.data)); }
        };
        ws.onclose = function () {
          if (!closed) {
            term.write(new TextEncoder().encode("\r\n\x1b[90m[disconnected - reconnecting]\x1b[0m\r\n"));
            setTimeout(connect, 1000);
          }
        };
      }
      term.onData(function (data) {
        if (ws && ws.readyState === 1) {
          ws.send(typeof data === "string" ? new TextEncoder().encode(data) : data);
        }
      });
      new ResizeObserver(function () {
        // Hidden tabs (display:none) must not fit to a 0x0 box.
        if (host.offsetParent !== null) {
          term.fit();
          sendResize();
        }
      }).observe(host);
      connect();
      window.__nsTerms = window.__nsTerms || {};
      window.__nsTerms[id] = {
        dispose: function () {
          closed = true;
          try { if (ws) { ws.close(); } } catch (_e) {}
          term.dispose();
          delete window.__nsTerms[id];
        },
      };
    })
    .catch(function (err) {
      host.textContent = "terminal failed to load: " + err;
    });
})();
