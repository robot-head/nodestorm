# WebView2: layout freeze after resizing while occluded

**Status:** WebView2 runtime defect (not nodestorm, not wry/tao/dioxus).
Reported upstream — link the issue here once filed.
**Observed:** 2026-07-17, Windows 11 Pro 26200, WebView2 150.0.4078.65,
150% DPI (144), nodestorm v0.8 stack (dioxus-desktop 0.7.9 / wry 0.53.5 /
tao 0.34.8).

## Symptom

If the nodestorm window is resized by another process while it is fully
occluded (bottom of the z-order, covered by other windows) — e.g. by
FancyZones re-tiling, an RDP reconnect, or a monitor/DPI rearrangement —
the **first** such resize applies normally, but every **subsequent** one
resizes only the native windows: the page keeps the previous layout.
Symptoms: content cropped at the window edge (or letterboxed), and click
targets misaligned with the visible pixels. Bringing the window forward
does **not** repaint it correctly; neither does clicking.

**User-facing workaround:** resize the window meaningfully (grow it by
more than ~64 physical px, or maximize/restore). Interactive drag-resize
after actually clicking into the window has not been observed to stay
frozen (activation was outside the automated test envelope — see caveat).

## What the investigation established

Instrumented app + window-targeted probe matrix (4 rounds; full evidence
and protocol in the session debug report, summarized here):

| Lever, from frozen state | Result |
| --- | --- |
| tao `WindowEvent::Resized` delivery | delivered for EVERY frozen resize |
| `ICoreWebView2Controller.SetBounds` | executed; browser child HWNDs (incl. `Chrome_RenderWidgetHostHWND`) track every new size |
| Unocclude (window becomes visible), no resize | stays frozen |
| Posted in-page click (processed by the page) | stays frozen |
| Same-size `SetBounds` re-push (single or repeated, occluded or visible) | stays frozen |
| Visible resize +1 px / +32 px | stays frozen |
| Visible resize +64 px / +160 px (grow) | **commits** |
| Visible resize shrink (any size) | stays frozen — no shrink ever committed after the first freeze |
| Chromium switches (`CalculateNativeWinOcclusion` off, backgrounding trio) | no effect |

Conclusions: the app→WebView2 hand-off is fully healthy; the stall is in
the browser→renderer visual-property/surface sync for a hidden (and
never-activated) widget. The ≥ +64 px grow-only flush is consistent with
a buffer-reallocation path forcing a new surface while the ordinary
resize-ack path stays stuck. Closest public report:
[WebView2Feedback #2983](https://github.com/MicrosoftEdge/WebView2Feedback/issues/2983)
(resize-while-hidden stays blank until activation or another resize).

**Caveat:** the entire matrix ran without ever activating the window
(deliberately — the machine was in active use; all automation was
window-targeted PostMessage/UIA). Real user activation (a genuine click,
Alt-Tab) is the one untested lever and plausibly flushes — which would
explain why this is rarely seen outside automation.

## Why no app-level fix

Both candidate fixes are ruled out by evidence: `set_bounds` on `Resized`
already happens (dioxus does it; it ran on every frozen resize), and every
cheap flush (re-push, click, small jiggle) is inert while the only working
lever — a ≥ +64 px *grow* — cannot restore the true size afterwards
(shrink-back freezes, stranding the window oversized). Anything shippable
would be a speculative activation-emulation hack.

## Related: UIA BoundingRectangle corruption after keyboard zoom-reset

Found during demo recording (2026-07-18), same
WebView2-under-window-automation family: after posting the `0`
(zoom-to-fit) key to the render widget, subsequent UIA
`BoundingRectangle` queries for elements inside the WebView return
corrupted rects until further interaction, even though the page renders
and behaves correctly. Worked around in `scripts/record-demo.ps1` by
ordering rect-dependent actions (context-menu clicks, element-targeted
moves) before any keyboard zoom-reset — see the comments in
`RightClick-Element` and `Invoke-Segment4`. Not yet minimally reproduced
or reported upstream.

## E2E impact

`scripts/verify-windows.ps1` performs multiple background resizes and hits
this deterministically at 150% DPI. It works around it by clamping click
coordinates into the live render-widget client area (see the comment at
the "At <=560px" block) — a no-op on machines where the layout reflows.
