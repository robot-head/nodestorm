# App Icon Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the malformed A-like icon with a bold open lightning path whose three node points remain clear in the in-app topbar and every packaged desktop launcher.

**Architecture:** `src/icon.rs` owns the geometry and deterministic rasterizer. The Dioxus topbar renders those constants directly as inline SVG, while `examples/generate_icons.rs` emits committed SVG/PNG assets from the same constants and supports a byte-for-byte `--check` mode. Platform packaging consumes only those generated assets: Windows scales the 1024px tile, macOS builds an iconset and `.icns`, and Linux installs hicolor PNGs plus a desktop entry.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7.9, `image` 0.25.10 with PNG support, Node.js 24 tests, PowerShell/System.Drawing, macOS `iconutil`, freedesktop desktop entries and hicolor icons.

## Global Constraints

- The mark is an open, right-leaning three-segment lightning path; it must not form a triangle, enclosure, or letter-like silhouette.
- Circular nodes are fused to both endpoints and the main bend.
- The standalone in-app mark uses `currentColor`, remains decorative (`aria-hidden`), and preserves the existing 17px topbar footprint and responsive wordmark behavior.
- The OS-facing mark is white on a neutral charcoal rounded tile with generous padding, no glow, no decorative outline, and no platform-specific redraw.
- One geometry definition drives inline SVG, generated SVG, native titlebar PNG, Windows assets, macOS `.icns`, and Linux hicolor PNGs.
- Icon generation is deterministic, local, and network-free.
- Do not change app behavior, menus, or window sizing.

---

### Task 1: Canonical geometry, deterministic assets, and topbar mark

**Files:**
- Create: `src/icon.rs`
- Create: `examples/generate_icons.rs`
- Create: `assets/icons/nodestorm-mark.svg`
- Create: `assets/icons/nodestorm-tile.svg`
- Create: `assets/icons/nodestorm-16.png`
- Create: `assets/icons/nodestorm-32.png`
- Create: `assets/icons/nodestorm-48.png`
- Create: `assets/icons/nodestorm-64.png`
- Create: `assets/icons/nodestorm-128.png`
- Create: `assets/icons/nodestorm-256.png`
- Create: `assets/icons/nodestorm-512.png`
- Create: `assets/icons/nodestorm-1024.png`
- Delete: `assets/nodestorm-icon.png`
- Modify: `src/lib.rs`
- Modify: `src/ui/topbar.rs:54-76`
- Modify: `src/ui/mod.rs:24`
- Modify: `assets/main.css:93-99`
- Modify: `src/theme.rs:181-196`

**Interfaces:**
- Produces: `icon::VIEW_BOX`, `icon::BOLT_POINTS`, `icon::NODE_INDICES`, `icon::STROKE_WIDTH`, `icon::STROKE_RADIUS`, `icon::NODE_RADIUS`, `icon::svg_points()`, `icon::mark_contains()`, and `icon::render_tile(size)`.
- Consumers: the topbar reads the geometry constants; the generator writes every SVG/PNG asset; native Tao decoding includes `assets/icons/nodestorm-256.png`.

- [ ] **Step 1: Add failing geometry and branding tests**

  Add `pub mod icon;` to `src/lib.rs`, then create `src/icon.rs` with tests only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn bolt_is_open_and_nodes_are_on_the_path() {
          assert_eq!(BOLT_POINTS.len(), 4);
          assert_ne!(BOLT_POINTS.first(), BOLT_POINTS.last());
          assert_eq!(NODE_INDICES, [0, 2, 3]);
          for index in NODE_INDICES {
              assert!(index < BOLT_POINTS.len());
          }
      }

      #[test]
      fn tile_is_square_rgba_with_transparent_corners() {
          let tile = render_tile(64);
          assert_eq!(tile.dimensions(), (64, 64));
          assert_eq!(tile.get_pixel(0, 0).0[3], 0);
          assert_eq!(tile.get_pixel(32, 32).0[3], 255);
      }
  }
  ```

  Replace the old `topbar_uses_the_shared_graph_bolt_mark` assertions in `src/theme.rs` with:

  ```rust
  assert!(brand.contains("polyline"));
  assert!(brand.contains("BOLT_POINTS"));
  assert!(brand.contains("NODE_INDICES"));
  assert!(brand.contains("currentColor"));
  assert!(!brand.contains("mask"));
  assert!(!brand.contains("topbar-bolt-cutout"));
  assert!(!brand.contains("\"ϟ\""));
  ```

- [ ] **Step 2: Run the tests and verify RED**

  Run: `cargo test icon::tests`

  Run: `cargo test topbar_uses_the_shared_graph_bolt_mark`

  Expected: the first command fails because the geometry constants and `render_tile` do not exist. The second fails because the topbar still contains the masked A-like mark.

- [ ] **Step 3: Implement the canonical geometry and renderer**

  Implement `src/icon.rs` with these exact public constants and helpers:

  ```rust
  use image::{Rgba, RgbaImage};

  pub const VIEW_BOX: &str = "0 0 256 256";
  pub const BOLT_POINTS: [(f32, f32); 4] = [
      (170.0, 34.0),
      (88.0, 112.0),
      (150.0, 112.0),
      (70.0, 222.0),
  ];
  pub const NODE_INDICES: [usize; 3] = [0, 2, 3];
  pub const STROKE_WIDTH: f32 = 30.0;
  pub const STROKE_RADIUS: f32 = STROKE_WIDTH / 2.0;
  pub const NODE_RADIUS: f32 = 20.0;
  pub const TILE_BG: [u8; 4] = [36, 39, 45, 255];

  pub fn svg_points() -> String {
      BOLT_POINTS
          .iter()
          .map(|(x, y)| format!("{x:.0},{y:.0}"))
          .collect::<Vec<_>>()
          .join(" ")
  }

  fn distance_sq_to_segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
      let ab = (b.0 - a.0, b.1 - a.1);
      let ap = (px - a.0, py - a.1);
      let length_sq = ab.0 * ab.0 + ab.1 * ab.1;
      let t = ((ap.0 * ab.0 + ap.1 * ab.1) / length_sq).clamp(0.0, 1.0);
      let dx = px - (a.0 + t * ab.0);
      let dy = py - (a.1 + t * ab.1);
      dx * dx + dy * dy
  }

  pub fn mark_contains(x: f32, y: f32) -> bool {
      BOLT_POINTS
          .windows(2)
          .any(|s| distance_sq_to_segment(x, y, s[0], s[1]) <= STROKE_RADIUS.powi(2))
          || NODE_INDICES.iter().any(|&i| {
              let (nx, ny) = BOLT_POINTS[i];
              (x - nx).powi(2) + (y - ny).powi(2) <= NODE_RADIUS.powi(2)
          })
  }
  ```

  Add this exact tile hit-test and 4×4 supersampled renderer. The tile and mark coverage are sampled independently; white mark samples replace charcoal tile samples, and uncovered samples remain transparent:

  ```rust
  fn rounded_tile_contains(x: f32, y: f32) -> bool {
      const MIN: f32 = 8.0;
      const MAX: f32 = 248.0;
      const RADIUS: f32 = 42.0;
      let nearest_x = x.clamp(MIN + RADIUS, MAX - RADIUS);
      let nearest_y = y.clamp(MIN + RADIUS, MAX - RADIUS);
      x >= MIN
          && x <= MAX
          && y >= MIN
          && y <= MAX
          && (x - nearest_x).powi(2) + (y - nearest_y).powi(2) <= RADIUS.powi(2)
  }

  pub fn render_tile(size: u32) -> RgbaImage {
      const SAMPLES: u32 = 4;
      let mut image = RgbaImage::new(size, size);
      for py in 0..size {
          for px in 0..size {
              let mut rgb = [0u32; 3];
              let mut covered = 0;
              for sy in 0..SAMPLES {
                  for sx in 0..SAMPLES {
                      let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32)
                          * 256.0 / size as f32;
                      let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32)
                          * 256.0 / size as f32;
                      let sample = if mark_contains(x, y) {
                          Some([255, 255, 255])
                      } else if rounded_tile_contains(x, y) {
                          Some([TILE_BG[0], TILE_BG[1], TILE_BG[2]])
                      } else {
                          None
                      };
                      if let Some(sample) = sample {
                          covered += 1;
                          for channel in 0..3 {
                              rgb[channel] += u32::from(sample[channel]);
                          }
                      }
                  }
              }
              let count = SAMPLES * SAMPLES;
              let pixel = if covered == 0 {
                  [0, 0, 0, 0]
              } else {
                  [
                      (rgb[0] / covered) as u8,
                      (rgb[1] / covered) as u8,
                      (rgb[2] / covered) as u8,
                      (covered * 255 / count) as u8,
                  ]
              };
              image.put_pixel(px, py, Rgba(pixel));
          }
      }
      image
  }
  ```

- [ ] **Step 4: Render the same geometry in the topbar**

  Replace the masked SVG in `src/ui/topbar.rs` with:

  ```rust
  let mark_points = crate::icon::svg_points();

  svg {
      class: "topbar-mark",
      view_box: crate::icon::VIEW_BOX,
      "aria-hidden": "true",
      polyline {
          points: "{mark_points}",
          fill: "none",
          stroke: "currentColor",
          stroke_width: "{crate::icon::STROKE_WIDTH}",
          stroke_linecap: "round",
          stroke_linejoin: "round",
      }
      for index in crate::icon::NODE_INDICES {
          circle {
              cx: "{crate::icon::BOLT_POINTS[index].0}",
              cy: "{crate::icon::BOLT_POINTS[index].1}",
              r: "{crate::icon::NODE_RADIUS}",
              fill: "currentColor",
          }
      }
  }
  ```

  Keep `.topbar-mark` at 17×17px with `color: var(--accent)`; do not add a background, mask, filter, outline, or glow.

- [ ] **Step 5: Implement deterministic SVG/PNG generation**

  In `examples/generate_icons.rs`, use `nodestorm::icon` to build:

  - `nodestorm-mark.svg`: transparent `<svg viewBox="0 0 256 256">` containing one rounded `polyline` and the three node circles in `currentColor`.
  - `nodestorm-tile.svg`: a 256-unit SVG containing a charcoal rounded rectangle and the same polyline/circles in white.
  - PNGs for `[16, 32, 48, 64, 128, 256, 512, 1024]` using `render_tile(size)`.

  Add helpers that return the SVG text as UTF-8 bytes, then build the desired output map before touching the filesystem:

  ```rust
  use image::ImageFormat;
  use nodestorm::icon;
  use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf};

  let check = std::env::args().skip(1).any(|arg| arg == "--check");
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
  let mut outputs = BTreeMap::new();
  outputs.insert("nodestorm-mark.svg".into(), mark_svg().into_bytes());
  outputs.insert("nodestorm-tile.svg".into(), tile_svg().into_bytes());
  for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
      let mut bytes = Vec::new();
      image::DynamicImage::ImageRgba8(icon::render_tile(size))
          .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
      outputs.insert(format!("nodestorm-{size}.png"), bytes);
  }

  if check {
      for (name, expected) in &outputs {
          let actual = fs::read(root.join(name))?;
          assert_eq!(&actual, expected, "stale generated asset: {name}");
      }
  } else {
      fs::create_dir_all(&root)?;
      for (name, bytes) in outputs {
          fs::write(root.join(name), bytes)?;
      }
  }
  ```

  `mark_svg()` must interpolate `icon::VIEW_BOX`, `icon::svg_points()`, `icon::STROKE_WIDTH`, `icon::NODE_RADIUS`, and all `NODE_INDICES`; use `stroke="currentColor"`, `stroke-linecap="round"`, and `stroke-linejoin="round"`. `tile_svg()` uses the same mark elements in white plus `<rect x="8" y="8" width="240" height="240" rx="42" fill="#24272D"/>`. End both SVGs with one newline so check mode is stable.

  Run: `cargo run --example generate_icons`

  Then change `APP_ICON_PNG` in `src/ui/mod.rs` to `include_bytes!("../../assets/icons/nodestorm-256.png")` and delete `assets/nodestorm-icon.png`.

- [ ] **Step 6: Verify GREEN and generated-asset stability**

  Run: `cargo test icon::tests`

  Run: `cargo test topbar_uses_the_shared_graph_bolt_mark`

  Expected: all three focused tests pass.

  Run: `cargo run --example generate_icons -- --check`

  Expected: exit 0 with every committed SVG/PNG byte-for-byte current.

- [ ] **Step 7: Commit the new visual foundation**

  ```bash
  git add src/icon.rs src/lib.rs src/ui/topbar.rs src/ui/mod.rs src/theme.rs assets/main.css examples/generate_icons.rs assets/icons assets/nodestorm-icon.png
  git commit -m "feat(brand): replace graph-bolt icon"
  ```

### Task 2: Native titlebar and Windows launcher assets

**Files:**
- Modify: `src/ui/mod.rs:204-215`
- Modify: `packaging/windows/prepare-layout.ps1:18-37`
- Modify: `tests/release_gates.mjs`

**Interfaces:**
- Consumes: `assets/icons/nodestorm-256.png` for Tao and `assets/icons/nodestorm-1024.png` for Windows raster scaling.
- Produces: validated native icon construction plus StoreLogo, Square44x44Logo, Square150x150Logo, and centered Wide310x150Logo package assets.

- [ ] **Step 1: Add failing Windows source-contract tests**

  Add a `release_gates.mjs` test that loads `packaging/windows/prepare-layout.ps1` and asserts:

  ```js
  assert.match(script, /assets[\\/]icons[\\/]nodestorm-1024\.png/i);
  assert.doesNotMatch(script, /docs[\\/]demo[\\/]poster\.png/i);
  assert.match(script, /Wide310x150Logo\.png/);
  assert.match(script, /X\s*=\s*\(\$asset\.Width\s*-\s*\$side\)\s*\/\s*2/);
  ```

- [ ] **Step 2: Run the test and verify RED**

  Run: `node --test tests/release_gates.mjs`

  Expected: the new test fails because the PowerShell script still reads `docs/demo/poster.png` and stretches it.

- [ ] **Step 3: Switch Windows layout generation to the tile master**

  In `prepare-layout.ps1`, load `../../assets/icons/nodestorm-1024.png`. For square assets draw it edge-to-edge with high-quality bicubic interpolation. For `Wide310x150Logo.png`, create a transparent 310×150 bitmap, set `$side = 150`, `$x = ($asset.Width - $side) / 2`, and draw the square source at `$x, 0, $side, $side` so it is centered rather than distorted.

  Preserve the existing manifest names and the Store `BackgroundColor="transparent"` contract.

- [ ] **Step 4: Verify Windows and Tao contracts**

  Run: `node --test tests/release_gates.mjs`

  Expected: all release-gate tests pass.

  Run: `cargo test embedded_app_icon`

  Expected: the 256px RGBA and Tao icon-construction tests pass against the redesigned PNG.

- [ ] **Step 5: Commit Windows integration**

  ```bash
  git add packaging/windows/prepare-layout.ps1 tests/release_gates.mjs src/ui/mod.rs
  git commit -m "feat(windows): package redesigned icon"
  ```

### Task 3: macOS `.icns` launcher integration

**Files:**
- Modify: `packaging/macos/Info.plist`
- Modify: `.github/workflows/release-build.yml:123-135`
- Modify: `tests/release_gates.mjs`

**Interfaces:**
- Consumes: committed PNG sizes 16, 32, 64, 128, 256, 512, and 1024.
- Produces: `Nodestorm.icns` in `Contents/Resources`, referenced by `CFBundleIconFile`.

- [ ] **Step 1: Add failing macOS packaging tests**

  Add source-contract assertions:

  ```js
  assert.match(plist, /<key>CFBundleIconFile<\/key><string>Nodestorm\.icns<\/string>/);
  for (const name of [
    "icon_16x16.png", "icon_16x16@2x.png",
    "icon_32x32.png", "icon_32x32@2x.png",
    "icon_128x128.png", "icon_128x128@2x.png",
    "icon_256x256.png", "icon_256x256@2x.png",
    "icon_512x512.png", "icon_512x512@2x.png",
  ]) assert.match(workflow, new RegExp(name.replaceAll(".", "\\.")));
  assert.match(workflow, /iconutil -c icns/);
  assert.match(workflow, /Contents\/Resources\/Nodestorm\.icns/);
  ```

- [ ] **Step 2: Run the test and verify RED**

  Run: `node --test tests/release_gates.mjs`

  Expected: the new macOS assertions fail because the plist and bundle do not contain an icon.

- [ ] **Step 3: Build the macOS iconset in the release workflow**

  Add `CFBundleIconFile` with value `Nodestorm.icns` to `Info.plist`. In the macOS bundle step, create `$RUNNER_TEMP/Nodestorm.iconset` and `Contents/Resources`, copy the exact generated sizes into Apple's standard iconset names, then run:

  ```bash
  iconutil -c icns -o "$APP/Contents/Resources/Nodestorm.icns" "$ICONSET"
  test -s "$APP/Contents/Resources/Nodestorm.icns"
  ```

  Use the 32px file for both `icon_16x16@2x.png` and `icon_32x32.png`, the 256px file for both `icon_128x128@2x.png` and `icon_256x256.png`, and the 512px file for both `icon_256x256@2x.png` and `icon_512x512.png`.

- [ ] **Step 4: Verify the macOS source contract**

  Run: `node --test tests/release_gates.mjs`

  Expected: all release-gate tests pass.

- [ ] **Step 5: Commit macOS integration**

  ```bash
  git add packaging/macos/Info.plist .github/workflows/release-build.yml tests/release_gates.mjs
  git commit -m "feat(macos): package app icon"
  ```

### Task 4: Linux hicolor icons and desktop launcher

**Files:**
- Modify: `.github/workflows/release-build.yml:82-88`
- Modify: `plugins/nodestorm/skills/nodestorm/scripts/setup.sh:139-163`
- Modify: `tests/installers.mjs`
- Modify: `tests/release_gates.mjs`

**Interfaces:**
- Consumes: `assets/icons/nodestorm-{128,256,512}.png` from Task 1.
- Produces: archive entries under `icons/<size>x<size>/nodestorm.png`, installed hicolor icons, and `$XDG_DATA_HOME/applications/nodestorm.desktop` with `Icon=nodestorm` and an absolute `Exec` path.

- [ ] **Step 1: Extend the Linux fixture and add a failing success-path test**

  Extend the existing `node:fs/promises` import with `access` and `copyFile`. In `linuxFailureFixture`, create `staging/icons/{128x128,256x256,512x512}`, copy the matching committed PNG into each directory as `nodestorm.png`, add `chmod` to the linked test commands, and change the tar inputs from just `"nodestorm"` to `"nodestorm", "icons"`. Then add:

  ```js
  test("Linux setup installs launcher and hicolor icons", async () => {
    const fixture = await linuxFailureFixture();
    const result = spawnSync(
      "/bin/bash",
      [path.join(scripts, "setup.sh"), "--os", "linux", "--arch", "x64", "--approve-install", "--skip-launch"],
      { env: fixture.env, encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    const data = fixture.env.XDG_DATA_HOME;
    assert.equal(await readFile(path.join(data, "applications", "nodestorm.desktop"), "utf8").then((s) => s.includes("Icon=nodestorm")), true);
    for (const size of [128, 256, 512]) {
      await access(path.join(data, "icons", "hicolor", `${size}x${size}`, "apps", "nodestorm.png"));
    }
  });
  ```

- [ ] **Step 2: Run the test and verify RED**

  Run: `node --test tests/installers.mjs`

  Expected: the new success-path test fails because setup installs only the binary.

- [ ] **Step 3: Package and install the Linux launcher artwork**

  In the Linux workflow, create `dist/icons/{128x128,256x256,512x512}` and copy the generated PNGs as `nodestorm.png`; include `icons` in the tar command.

  In `setup.sh`, after installing the binary:

  ```bash
  for size in 128 256 512; do
    staged_icon="$TEMP_DIR/icons/${size}x${size}/nodestorm.png"
    [[ -f "$staged_icon" ]] || { echo "Release archive has no ${size}px launcher icon." >&2; exit 1; }
    icon_dir="${XDG_DATA_HOME:-${HOME}/.local/share}/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$icon_dir"
    install -m 0644 "$staged_icon" "$icon_dir/nodestorm.png"
  done

  desktop_dir="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
  mkdir -p "$desktop_dir"
  desktop_exec="$INSTALL_DIR/nodestorm"
  desktop_exec=${desktop_exec//\\/\\\\}
  desktop_exec=${desktop_exec//\"/\\\"}
  desktop_exec=${desktop_exec//\$/\\$}
  desktop_exec=${desktop_exec//\`/\\\`}
  {
    printf '[Desktop Entry]\nType=Application\nVersion=1.0\n'
    printf 'Name=Nodestorm\nComment=Visual architecture brainstorming\n'
    printf 'Exec="%s"\nIcon=nodestorm\nTerminal=false\nCategories=Development;\n' "$desktop_exec"
  } > "$desktop_dir/nodestorm.desktop"
  chmod 0644 "$desktop_dir/nodestorm.desktop"
  ```

  Add release-gate source assertions that the Linux tar includes `icons` and the setup script contains `Icon=nodestorm` plus all three hicolor sizes.

- [ ] **Step 4: Verify Linux installation and release contracts**

  Run: `node --test tests/installers.mjs tests/release_gates.mjs`

  Expected: all installer and release-gate tests pass, including the new launcher success path.

- [ ] **Step 5: Commit Linux integration**

  ```bash
  git add .github/workflows/release-build.yml plugins/nodestorm/skills/nodestorm/scripts/setup.sh tests/installers.mjs tests/release_gates.mjs
  git commit -m "feat(linux): install desktop launcher"
  ```

### Task 5: Full verification and visual acceptance

**Files:**
- Modify: none

**Interfaces:**
- Consumes: all geometry, generated assets, and platform packaging from Tasks 1-4.
- Produces: verification evidence only.

- [ ] **Step 1: Verify generated assets are current**

  Run: `cargo run --example generate_icons -- --check`

  Expected: exit 0 with no stale asset report.

- [ ] **Step 2: Run formatting and complete automated suites**

  Run: `cargo fmt --all -- --check`

  Expected: exit 0.

  Run: `cargo test --all-targets --locked`

  Expected: exit 0 with no failed tests.

  Run: `node --test tests/installers.mjs tests/release_gates.mjs`

  Expected: exit 0 with no failed tests.

  Run: `cargo build --locked`

  Expected: exit 0.

- [ ] **Step 3: Inspect the generated artwork directly**

  Open `assets/icons/nodestorm-16.png`, `nodestorm-256.png`, and `nodestorm-1024.png` at native scale. Confirm the zigzag reads as a bolt, all three nodes remain visible, no triangle/A silhouette appears, the corners are transparent, and the tile has balanced padding.

- [ ] **Step 4: Inspect the running desktop app**

  Run: `cargo run -- --demo`

  Confirm the standalone mark reads clearly at 17px in the topbar, remains when the wordmark hides below 600px, and the native window icon is legible against available light and dark titlebar modes.

- [ ] **Step 5: Inspect platform package output where available**

  On Windows, run `packaging/windows/prepare-layout.ps1` and inspect all four PNGs without wide-logo distortion. On macOS, inspect `Nodestorm.icns` in the signed app bundle. On Linux, run the installer fixture and confirm the desktop launcher resolves `Icon=nodestorm` from hicolor.

- [ ] **Step 6: Check the final branch diff**

  Run: `git diff --check origin/main...HEAD`

  Expected: exit 0 with no whitespace errors.

## Platform references

- Windows logo manifest roles and supported scale variants: https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap-visualelements
- macOS iconset filenames and Retina pairs: https://developer.apple.com/library/archive/documentation/Xcode/Reference/xcode_ref-Asset_Catalog_Format/IconSetType.html
- macOS `iconutil` packaging and bundle icon configuration: https://developer.apple.com/library/archive/documentation/GraphicsAnimation/Conceptual/HighResolutionOSX/Optimizing/Optimizing.html
- freedesktop desktop-entry `Name`, `Exec`, and `Icon` fields: https://specifications.freedesktop.org/desktop-entry/latest-single/
- hicolor third-party application icon layout: https://specifications.freedesktop.org/icon-theme/latest/index.html
