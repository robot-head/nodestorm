# App Icon and Titlebar Branding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a shared monochrome node-graph/lightning-bolt brand mark to Nodestorm's in-app topbar and native desktop window icon.

**Architecture:** `assets/nodestorm-mark.svg` is the canonical transparent graph-and-bolt silhouette. The topbar references it through a CSS mask, allowing the existing theme accent to color a single shared shape. A 256px transparent PNG version supplies Tao's native window icon; it is compiled into the executable and decoded to RGBA before the window is built.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7.9, Tao 0.34.8, CSS, SVG, PNG decoded with `image` 0.25.

## Global Constraints

- The mark is a compact monochrome graph with three circular nodes and a central lightning-bolt cutout.
- The app mark must remain readable in a narrow 48px topbar and on light or dark native titlebars.
- The in-app topbar keeps the existing `nodestorm` wordmark and responsive rule that hides it at 600px.
- The native PNG must be embedded with `include_bytes!`; do not read an icon from the runtime filesystem.
- Do not change application behaviour, menus, window sizing, or Windows Store tile artwork.

---

### Task 1: Add the shared graph-and-bolt mark to the topbar

**Files:**
- Create: `assets/nodestorm-mark.svg`
- Modify: `assets/main.css:84-101`
- Modify: `src/ui/topbar.rs:53-55`
- Modify: `src/theme.rs:128-170`

**Interfaces:**
- Consumes: `/assets/nodestorm-mark.svg`, served by Dioxus's desktop asset resolver.
- Produces: CSS class `.topbar-mark`, a decorative topbar element that uses the canonical SVG as a mask.

- [ ] **Step 1: Write the failing branding contract test**

  In `src/theme.rs`'s test module, load the SVG alongside the existing CSS and topbar fixtures and add this test:

  ```rust
  const APP_ICON_SVG: &str = include_str!("../assets/nodestorm-mark.svg");

  #[test]
  fn topbar_uses_the_shared_graph_bolt_mark() {
      assert!(TOPBAR_SOURCE.contains("topbar-mark"));
      assert!(!TOPBAR_SOURCE.contains("\"ϟ\""));
      assert!(APP_ICON_SVG.contains("viewBox=\"0 0 256 256\""));
      assert!(APP_ICON_SVG.contains("id=\"bolt-cutout\""));
      assert_block_contains(
          ".topbar-mark",
          "mask: url(\"/assets/nodestorm-mark.svg\") center / contain no-repeat",
      );
  }
  ```

- [ ] **Step 2: Run the focused test and verify it fails**

  Run: `cargo test topbar_uses_the_shared_graph_bolt_mark`

  Expected: compilation fails because `assets/nodestorm-mark.svg` and `.topbar-mark` do not exist.

- [ ] **Step 3: Create the canonical transparent SVG**

  Create `assets/nodestorm-mark.svg` with a black, three-node graph whose center is removed with a bolt-shaped mask. Keep the transparent canvas and view box exactly as below so the CSS mask and the PNG derivation share one silhouette:

  ```svg
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
    <defs>
      <mask id="bolt-cutout">
        <rect width="256" height="256" fill="white"/>
        <path d="M145 42 78 137h47l-14 77 67-97h-47l14-75Z" fill="black"/>
      </mask>
    </defs>
    <g fill="#000" stroke="#000" stroke-linecap="round" stroke-linejoin="round" mask="url(#bolt-cutout)">
      <path d="M59 184 120 66 199 171" fill="none" stroke-width="24"/>
      <circle cx="59" cy="184" r="27" stroke="none"/>
      <circle cx="120" cy="66" r="27" stroke="none"/>
      <circle cx="199" cy="171" r="27" stroke="none"/>
    </g>
  </svg>
  ```

- [ ] **Step 4: Replace the glyph with the masked shared mark**

  In `src/ui/topbar.rs`, replace the bolt span:

  ```rust
  span { class: "topbar-bolt", "ϟ" }
  ```

  with a decorative span:

  ```rust
  span { class: "topbar-mark", aria_hidden: "true" }
  ```

  In `assets/main.css`, replace `.topbar-bolt` with this rule and center the brand items (not baseline-align them):

  ```css
  .topbar-brand {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    font-size: 14px;
  }

  .topbar-mark {
    display: inline-block;
    flex: 0 0 17px;
    width: 17px;
    height: 17px;
    background: var(--accent);
    -webkit-mask: url("/assets/nodestorm-mark.svg") center / contain no-repeat;
    mask: url("/assets/nodestorm-mark.svg") center / contain no-repeat;
  }
  ```

- [ ] **Step 5: Run the focused test and verify it passes**

  Run: `cargo test topbar_uses_the_shared_graph_bolt_mark`

  Expected: `test theme::tests::topbar_uses_the_shared_graph_bolt_mark ... ok`.

- [ ] **Step 6: Commit the independently testable topbar mark**

  ```bash
  git add assets/nodestorm-mark.svg assets/main.css src/theme.rs src/ui/topbar.rs
  git commit -m "feat(brand): add shared graph-bolt mark"
  ```

### Task 2: Embed the native titlebar icon

**Files:**
- Create: `assets/nodestorm-icon.png`
- Modify: `Cargo.toml:10-29`
- Modify: `src/ui/mod.rs:35-185`

**Interfaces:**
- Consumes: `assets/nodestorm-icon.png` as a 256×256 RGBA PNG.
- Produces: `fn app_icon() -> dioxus::desktop::tao::window::Icon`, passed to `WindowBuilder::with_window_icon`.

- [ ] **Step 1: Write the failing embedded-icon tests**

  Add these tests to `src/ui/mod.rs`'s existing `tests` module:

  ```rust
  #[test]
  fn embedded_app_icon_is_a_256px_rgba_png() {
      let image = image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png)
          .expect("embedded app icon must be a valid PNG");
      assert_eq!((image.width(), image.height()), (256, 256));
      assert_eq!(image.color(), image::ColorType::Rgba8);
  }

  #[test]
  fn embedded_app_icon_builds_a_tao_icon() {
      let _icon = app_icon();
  }
  ```

- [ ] **Step 2: Run the focused tests and verify they fail**

  Run: `cargo test embedded_app_icon`

  Expected: compilation fails because `APP_ICON_PNG`, `app_icon`, and the direct `image` dependency do not yet exist.

- [ ] **Step 3: Create the PNG app-icon rendition**

  Render the canonical SVG at 256×256 into `assets/nodestorm-icon.png`. The PNG must preserve alpha, use the same three-node/bolt composition, and include a dark charcoal rounded-square backing plus a thin white contour around the graph. This gives the monochrome icon contrast on both light and dark system titlebars while retaining the same mark as the topbar.

  Confirm the source file before wiring it into Rust:

  ```bash
  file assets/nodestorm-icon.png
  identify -format '%wx%h %[channels]\n' assets/nodestorm-icon.png
  ```

  Expected: a 256×256 PNG with an alpha channel.

- [ ] **Step 4: Decode the compiled PNG and supply Tao's window builder**

  Add a direct, PNG-only dependency in `Cargo.toml`:

  ```toml
  image = { version = "0.25.10", default-features = false, features = ["png"] }
  ```

  In `src/ui/mod.rs`, add the embedded bytes and helper above `launch`:

  ```rust
  const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/nodestorm-icon.png");

  fn app_icon() -> dioxus::desktop::tao::window::Icon {
      let image = image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png)
          .expect("embedded app icon must be a valid PNG")
          .into_rgba8();
      let (width, height) = image.dimensions();
      dioxus::desktop::tao::window::Icon::from_rgba(image.into_raw(), width, height)
          .expect("embedded app icon must have valid RGBA dimensions")
  }
  ```

  Chain the result into the existing builder before `.with_theme(...)`:

  ```rust
  .with_window_icon(Some(app_icon()))
  ```

- [ ] **Step 5: Run the focused tests and verify they pass**

  Run: `cargo test embedded_app_icon`

  Expected: both embedded-icon tests pass.

- [ ] **Step 6: Commit the native titlebar icon**

  ```bash
  git add Cargo.toml Cargo.lock assets/nodestorm-icon.png src/ui/mod.rs
  git commit -m "feat(desktop): set native window icon"
  ```

### Task 3: Run integration checks and inspect the desktop rendering

**Files:**
- Modify: none

**Interfaces:**
- Consumes: the shared SVG mark and embedded native PNG from Tasks 1–2.
- Produces: evidence that the project builds, tests, and renders the agreed branding.

- [ ] **Step 1: Format the Rust changes**

  Run: `cargo fmt --check`

  Expected: exit status 0. If formatting is required, run `cargo fmt`, then rerun the check and commit only the formatting change with `style: format app icon integration`.

- [ ] **Step 2: Run the complete Rust test suite**

  Run: `cargo test`

  Expected: exit status 0 with no failed tests.

- [ ] **Step 3: Build the desktop binary**

  Run: `cargo build`

  Expected: exit status 0.

- [ ] **Step 4: Manually inspect the running app**

  Run: `cargo run -- --demo`

  Expected: the native window/titlebar shows the graph-and-bolt icon; the topbar shows the same mark before `nodestorm`; at a narrow width the wordmark hides while the mark remains visible. Verify the icon has clear contrast in the available system titlebar mode.

- [ ] **Step 5: Check the final diff and report the verification evidence**

  Run: `git diff HEAD~2..HEAD --check`

  Expected: exit status 0. Report the test, build, and visual-inspection results, including any platform-specific limitation encountered.
