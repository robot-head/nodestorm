# Vendored Ferroterm

- Source: author-deployed build at https://datanoisetv.github.io/ferroterm/
  (web/src ES modules + web/pkg wasm-bindgen artifacts), retrieved 2026-07-20.
  Upstream repo master was `f77d0c1de83b27457af6d318f7df8a744ec17305` at
  retrieval; the wasm build is a Pages deployment artifact and is not
  committed upstream. MIT.
- SHA-256 of every vendored file (`Get-FileHash -Algorithm SHA256`):
  ```
  d7a2de240cb6ebd1d855c8bdbabd1c7c59a3f2ad2d77089d0dca8ad0a7bf9329  ferroterm.d.ts
  9c9f662e27ef38d63959c74dea8cf589107c45faca3e1d37ab0a8eddccab75a5  pkg/ferroterm_wasm_bg.wasm
  d03e3495752f605c2b5822ac2b6c383aa00d27b9ad75292f7619722302868333  pkg/ferroterm_wasm.js
  a6a241ee58f5324b7171032a0017f33ac315fe3e55651b50ca38657b2535fd81  src/element.js
  2e6bceee693aacb329992a907189b5895a73dd11db63fe4f538f19d0e6047e6a  src/ferroterm.js
  4ce1f27f2a281c159362de39ac5cc48ab69d99abc88f44e1c52a769d02f1138b  src/index.js
  52c11918730a0d16f3033271ac3d2338f760587dacdf32df60b3ab410f5c4ac3  src/keycodes.js
  2217f66f8a59f76c4f4b067fd335ef9f1d238eb51637afb5f8f54cf15891c5be  src/links.js
  71afbbe1414c099b084af1b733a990b9c18bd60fbfb637fa14e03e01beb80a76  src/model.js
  b80c4c95732fdeaf7e0babfbe1aad37a150ae4362cc077048cf457fd7e99adf3  src/palette.js
  aec02c6a97dd5d0a34dd2be8f72a2fc3ee093a7e0d6f95acd1790608e0fbd5fb  src/renderer-canvas.js
  81ab7f79a7eedeb01d812bd46382f6f002caba5c57b4affd5ff99bc8759f2cae  src/renderer-webgl.js
  ```
- Served at runtime from the embedded loopback server under
  /terminal/assets/, never from a CDN. `ferroterm.d.ts` is reference only
  (not served — see `src/server/terminal_ws.rs`).
- Update by re-vendoring and refreshing the hashes.
- `mount.js` is ours: the per-tab glue template (see `src/ui/terminal_panel.rs`).
