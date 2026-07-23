# Microsoft Store Logo Assets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Produce seven upload-ready Microsoft Store PNGs that express nodestorm's approved Electric Topology identity from 2160 px poster art down to a 71 px app tile.

**Architecture:** Establish one stable bolt-and-three-node symbol first, then use it as the visual reference for square box art, poster art, and simplified tile variants. Use the built-in image-generation tool for every raster deliverable, save selected outputs under assets/store/, inspect them visually, and validate their local file metadata before committing.

**Tech Stack:** Built-in image_gen, local view_image, POSIX shell, file, find, Git

**Approved execution note:** The built-in generator returned a 1254 × 1254
source despite the exact-canvas request. The user approved deterministic
Lanczos3 resampling through a temporary Rust helper linked against the
already-built image crate. The helper lives under tmp/imagegen/, does not
change Cargo dependencies or product code, and is excluded from the commit.

## Global Constraints

- All deliverables are opaque PNGs and less than 50 MB each.
- Exact export dimensions are authoritative: 720 × 1080, 1440 × 2160, 1080 × 1080, 2160 × 2160, 300 × 300, 150 × 150, and 71 × 71.
- Background is nodestorm midnight #0f1117; primary mark is electric blue #6c9ef8 with pale-blue highlights.
- The symbol is one sharp lightning bolt integrated with exactly three connected graph nodes; silhouette, node count, and node placement stay consistent.
- Poster art alone includes the exact lowercase wordmark nodestorm in Space Grotesk.
- Essential poster content stays inside the central 70% of the canvas.
- Exclude screenshots, slogans, weather imagery, photorealism, crop-edge detail, watermarks, and third-party branding.
- Box art omits the wordmark and background network; tiles also omit the grid and ambient glow.
- Keep generation intermediates out of the committed deliverable set.

---

### Task 1: Establish the symbol and square box art

**Files:**
- Create: assets/store/box-art-2160x2160.png
- Create: assets/store/box-art-1080x1080.png

**Interfaces:**
- Consumes: docs/superpowers/specs/2026-07-19-microsoft-store-logo-assets-design.md
- Produces: the canonical symbol used as the visual reference by Tasks 2 and 3

- [ ] **Step 1: Create the destination**

Run:

~~~bash
mkdir -p assets/store
~~~

Expected: assets/store/ exists and has no new deliverables yet.

- [ ] **Step 2: Generate the 2160 px square master**

Call the built-in image_gen tool with no input image and this prompt:

~~~text
Use case: logo-brand
Asset type: Microsoft Store 1:1 box art, exact canvas 2160 x 2160 pixels
Primary request: Create the canonical nodestorm Electric Topology symbol: one sharp geometric lightning bolt integrated with exactly three connected graph nodes.
Scene/backdrop: opaque solid midnight #0f1117 with only a very subtle sparse dot grid; no network illustration.
Subject: one centered, large bolt-and-three-node symbol with a stable compact silhouette and generous safe space on every side.
Style/medium: polished geometric raster logo with vector-clean edges and a professional developer-tool identity.
Lighting/mood: restrained electric energy; hard readable edges; minimal controlled halo.
Color palette: electric blue #6c9ef8, pale-blue edge highlight, midnight #0f1117.
Text: none.
Constraints: exactly three circular nodes; all three visibly connected to the bolt; opaque background; immediately readable at thumbnail size.
Avoid: letters, words, screenshots, cards, clouds, rain, photorealism, excessive glow, gradients near crop edges, extra nodes, watermark, third-party branding.
~~~

Save the selected output as assets/store/box-art-2160x2160.png.

- [ ] **Step 3: Inspect and correct the master**

Open assets/store/box-art-2160x2160.png with view_image at original detail.

Expected: one centered bolt, exactly three connected nodes, no text or cloud imagery, crisp edges, subtle grid, and broad crop safety.

If one requirement fails, make one targeted image_gen edit with this image as the edit target. State only the failed requirement and preserve the exact silhouette, palette, framing, and every correct detail.

- [ ] **Step 4: Produce matching 1080 px box art**

Use the approved 2160 px master as the reference image:

~~~text
Use case: logo-brand
Asset type: Microsoft Store 1:1 box art, exact canvas 1080 x 1080 pixels
Primary request: Reproduce the reference artwork exactly at the requested canvas size.
Input images: Image 1 is the canonical visual reference.
Constraints: preserve the bolt silhouette, exactly three node positions, connections, colors, background, grid density, centering, and safe space; no text; opaque PNG.
Avoid: redesign, additional detail, new symbols, watermark.
~~~

Save as assets/store/box-art-1080x1080.png. Inspect both square files side by side.

Expected: both show the same symbol and composition.

### Task 2: Produce both poster sizes

**Files:**
- Create: assets/store/poster-1440x2160.png
- Create: assets/store/poster-720x1080.png

**Interfaces:**
- Consumes: assets/store/box-art-2160x2160.png
- Produces: full-wordmark Store and Xbox poster art

- [ ] **Step 1: Generate the 1440 × 2160 poster**

Use the square master as a reference image:

~~~text
Use case: ads-marketing
Asset type: Microsoft Store poster art, exact canvas 1440 x 2160 pixels
Primary request: Create portrait nodestorm Store art using the exact Electric Topology symbol from the reference.
Input images: Image 1 is the canonical symbol reference; reproduce its silhouette, three nodes, connections, and palette without redesign.
Scene/backdrop: opaque midnight #0f1117 with a sparse dark blue-gray dot grid and a restrained abstract node network behind the subject.
Subject: symbol centered slightly above the visual midpoint; lowercase wordmark beneath it.
Style/medium: polished geometric developer-tool key art with vector-clean logo edges.
Composition/framing: symbol and wordmark entirely inside the central 70%; outer 15% on every edge contains background only.
Lighting/mood: controlled electric-blue energy, confident and technical.
Color palette: #0f1117 background, #6c9ef8 mark, pale-blue highlights, dark blue-gray paths.
Text (verbatim): "nodestorm"
Typography: lowercase Space Grotesk geometric sans, medium-bold, generous tracking, centered and fully legible.
Constraints: exact spelling n-o-d-e-s-t-o-r-m; exactly three nodes; opaque background; background network remains subordinate.
Avoid: other text, slogan, screenshot, UI cards, clouds, rain, photorealism, crop-edge detail, excessive glow, watermark, third-party branding.
~~~

Save as assets/store/poster-1440x2160.png.

- [ ] **Step 2: Inspect text and crop safety**

Open the poster with view_image at original detail and as a normal preview.

Expected: the word reads exactly nodestorm; the symbol and wordmark stay in the central 70%; background detail remains quiet.

If spelling alone is wrong, make one precise edit: replace only the wordmark with exact lowercase nodestorm in Space Grotesk and preserve every other design choice.

- [ ] **Step 3: Produce matching 720 × 1080 poster**

Use the approved large poster as the reference:

~~~text
Use case: ads-marketing
Asset type: Microsoft Store poster art, exact canvas 720 x 1080 pixels
Primary request: Reproduce the reference poster exactly at the requested canvas size.
Input images: Image 1 is the approved poster reference.
Text (verbatim): "nodestorm"
Constraints: preserve exact spelling, symbol silhouette, exactly three nodes, composition, central-70-percent safe area, palette, grid, network, and opaque background.
Avoid: redesign, new detail, changed typography, watermark.
~~~

Save as assets/store/poster-720x1080.png and inspect both posters together.

Expected: one matching composition and two readable wordmarks.

### Task 3: Produce purpose-built app tile icons

**Files:**
- Create: assets/store/app-tile-300x300.png
- Create: assets/store/app-tile-150x150.png
- Create: assets/store/app-tile-71x71.png

**Interfaces:**
- Consumes: assets/store/box-art-2160x2160.png
- Produces: three symbol-only opaque Store tiles optimized for native-size readability

- [ ] **Step 1: Generate the 300 px tile**

Use the canonical box-art master as the reference:

~~~text
Use case: logo-brand
Asset type: Microsoft Store app tile icon, exact canvas 300 x 300 pixels
Primary request: Adapt the canonical reference symbol into a small app tile without changing its identity.
Input images: Image 1 is the canonical symbol reference.
Scene/backdrop: flat opaque midnight #0f1117; no grid and no network.
Subject: centered bolt with exactly the same three-node arrangement, slightly thickened connections, generous padding.
Style/medium: crisp flat geometric icon with a restrained pale-blue inner highlight.
Constraints: no text; no ambient glow; preserve silhouette and node placement; readable at native size.
Avoid: extra detail, extra nodes, edge gradients, watermark.
~~~

Save as assets/store/app-tile-300x300.png and inspect at native size.

- [ ] **Step 2: Generate the 150 px tile**

Use the approved 300 px tile as the reference:

~~~text
Use case: logo-brand
Asset type: Microsoft Store app tile icon, exact canvas 150 x 150 pixels
Primary request: Reproduce the reference tile as a smaller app icon without changing its identity.
Input images: Image 1 is the approved 300 px tile reference.
Scene/backdrop: flat opaque midnight #0f1117; no grid and no network.
Subject: centered bolt with exactly the same three-node arrangement; connections slightly thicker than the reference for 150-pixel clarity; generous padding.
Style/medium: crisp flat geometric icon with a restrained pale-blue inner highlight.
Constraints: no text; no ambient glow; preserve silhouette and node placement; readable at native size.
Avoid: extra detail, extra nodes, edge gradients, watermark.
~~~

Save as assets/store/app-tile-150x150.png.

Expected: the bolt and all three nodes remain distinct with no glow haze.

- [ ] **Step 3: Generate the 71 px tile**

Use the approved 150 px tile as the reference:

~~~text
Use case: logo-brand
Asset type: Microsoft Store app tile icon, exact canvas 71 x 71 pixels
Primary request: Produce the smallest flat rendition of the reference symbol.
Input images: Image 1 is the approved 150 px tile reference.
Scene/backdrop: flat opaque midnight #0f1117.
Subject: centered electric-blue bolt and exactly three connected circular nodes, with thicker geometry suitable for 71 pixels.
Style/medium: flat high-contrast geometric icon; no glow, grid, network, texture, or fine highlight.
Constraints: preserve the canonical silhouette and node placement; each connection stays separated and visible; no text.
Avoid: tiny detail, blur, extra nodes, watermark.
~~~

Save as assets/store/app-tile-71x71.png and inspect at original size.

Expected: the mark immediately reads as a bolt-and-node symbol rather than a blue blob.

### Task 4: Validate and commit the Store set

**Files:**
- Test: assets/store/*.png

**Interfaces:**
- Consumes: all seven deliverables from Tasks 1–3
- Produces: one verified, committed Microsoft Store asset set

- [ ] **Step 1: Verify PNG type and exact dimensions**

Run:

~~~bash
file assets/store/*.png
~~~

Expected: seven PNG lines with these dimensions:

~~~text
app-tile-150x150.png: 150 x 150
app-tile-300x300.png: 300 x 300
app-tile-71x71.png: 71 x 71
box-art-1080x1080.png: 1080 x 1080
box-art-2160x2160.png: 2160 x 2160
poster-1440x2160.png: 1440 x 2160
poster-720x1080.png: 720 x 1080
~~~

If an image has the wrong dimensions but the correct aspect ratio, use the
approved temporary Lanczos3 resampler to export the exact size. If its aspect
ratio is wrong, regenerate it using the approved larger counterpart as the
visual reference; do not stretch a wrong-aspect composition.

- [ ] **Step 2: Verify count and size ceiling**

Run:

~~~bash
find assets/store -maxdepth 1 -type f -name '*.png' -printf '%f %s bytes\n' | sort
~~~

Expected: exactly seven files and every byte count below 52428800.

- [ ] **Step 3: Perform final visual comparison**

Open all seven files with view_image.

Expected:

- Every asset uses one recognizable symbol with exactly three nodes.
- Only the posters contain the exact word nodestorm.
- Both posters share one composition and both box-art files share one composition.
- Tiles have opaque backgrounds and progressively simpler effects.
- The 71 px tile remains crisp at native size.
- No asset contains accidental text, a watermark, a screenshot, cloud or rain imagery, or alpha fringe.

- [ ] **Step 4: Check the diff and commit**

Run:

~~~bash
git status --short
git diff --check
git add assets/store docs/superpowers/plans/2026-07-19-microsoft-store-logo-assets.md
git diff --cached --check
git commit -m "feat(store): add listing logo assets"
~~~

Expected: one commit containing the seven PNGs and this plan; the design spec remains unchanged.
