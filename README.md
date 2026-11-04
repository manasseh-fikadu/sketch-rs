# sketch-rs

A fork of [Sketch](https://github.com/michaelshimeles/sketch) by Michael Shimeles, a native Rust
infinite canvas built on `wgpu` + `winit` + `egui`. The app, its binaries and its folders keep the
name Sketch (`sketch-rs`), which keeps upstream's fixes easy to pull in.

This fork is for designing real UI with AI agents. Agents write HTML and CSS over MCP, and the
goal is that the canvas matches what a browser would render. `tests/fidelity.rs` measures that
against Chromium.

![Sketch main window](docs/screenshots/app_demo.png)

A local-first design and whiteboard tool. Projects are folders of pages; each page is an
infinite canvas with frames, groups, components and instances, auto layout and constraints,
vector paths and booleans, gradients and image fills, rich text, smart guides, and
pressure-sensitive ink, plus a whiteboard mode: hand-drawn (rough.js-style) shapes, arrows that
bind to shapes and re-route, elbow connectors, sticky notes, a laser pointer and a shape
library. Everything autosaves, and exports to PNG, SVG and PDF; Excalidraw files and SVG import.

![Figma-core features](docs/screenshots/milestone3_figma_core.png)

> **Origin.** Upstream describes Sketch as a reimplementation of the behavior of
> [Paper](https://paper.design), reverse engineered from Paper's desktop app. It says it contains
> no Paper code or assets. This repository starts from upstream commit `af814b7`, without
> upstream's git history or its notes on Paper's internals. This project isn't affiliated with
> Paper. Work in this fork is measured against CSS and Chromium, not Paper. The UI and canvas text
> use [Inter](https://rsms.me/inter/) 4.1 (SIL OFL 1.1), bundled in `assets/fonts/`.

To pick up an upstream fix:

```sh
git remote add upstream https://github.com/michaelshimeles/sketch.git
git fetch upstream
git log af814b7..upstream/main    # what's new since this fork started
git cherry-pick <commit>
```

## Projects

Sketch opens on a **home screen** of your projects (recents grid with thumbnails, New, Import,
and Rename / Duplicate / Move to Trash from each card's ⋯ menu or right-click; restore from
Trash). Each project is a plain folder in `~/Documents/sketch-rs/<name>/` holding `project.json`
and one JSON file per page in `pages/`. Work is autosaved about a second after each change with
crash-safe atomic writes; files are versioned and older versions migrate on load. Set
`SKETCH_RS_HOME` to use another folder.

Sketch used to be called paper-rs. On first launch it moves `~/Documents/paper-rs` to
`~/Documents/sketch-rs`, and it still opens `.paper-rs` files and older libraries. The old
`PAPER_RS_*` environment variables still work when the matching `SKETCH_RS_*` one isn't set.

A project has any number of **pages** (Pages list at the top of the left sidebar: + to add,
double-click to rename, right-click to duplicate or delete, drag to reorder). Each page is its
own infinite canvas with its own undo history and view. Copy and paste works between pages and
between projects.

## Build and run

You need Rust 1.95 or newer (`rust-version` in `Cargo.toml`; the latest stable is recommended). Install it with [rustup](https://rustup.rs).

```sh
cargo build --release
./target/release/sketch-rs                  # projects home screen
./target/release/sketch-rs --demo           # sample planning board (not saved)
./target/release/sketch-rs ~/Documents/sketch-rs/Plan   # open a project folder
./target/release/sketch-rs board.sketch-rs   # open a Sketch 0.1 document
```

### macOS

```sh
xcode-select --install          # command line tools (linker)
curl https://sh.rustup.rs -sSf | sh
cargo build --release && ./target/release/sketch-rs --demo
```

It uses Metal through wgpu. Shortcuts use ⌘, and trackpad pinch zooms. This fork has no signed
app bundle or auto-updater: upstream's were tied to its own releases and signing key, so they were
removed. Run it from the build folder.

### Linux

```sh
# Debian/Ubuntu runtime libraries (usually already present on desktops)
sudo apt install libxkbcommon-x11-0 libxcb-xkb1 libvulkan1 mesa-vulkan-drivers
cargo build --release && ./target/release/sketch-rs --demo
```

It runs on X11 and Wayland through Vulkan, or GL as a fallback. Shortcuts use Ctrl. File
dialogs use the XDG desktop portal.

### Headless (servers, CI)

No display is needed. wgpu renders offscreen, through Mesa lavapipe when there is no GPU:

```sh
sketch-rs --screenshot ui.png --demo --size 1440x900 --scale 2      # full UI
sketch-rs --screenshot home.png --home                              # projects home screen
sketch-rs --export-png out.png PROJECT_DIR --page 0 --scale 2 [--transparent]
sketch-rs --export-svg out.svg PROJECT_DIR
sketch-rs --export-pdf out.pdf PROJECT_DIR --frames                 # one PDF page per frame
sketch-rs --export out_dir PROJECT_DIR --format png --scope frames  # a folder of PNGs
sketch-rs --import board.excalidraw                                 # new project, prints its folder
```

## AI agents (MCP)

![A sign-in screen written by an agent through write_html](docs/screenshots/milestone7_mcp.png)

Sketch serves the [Model Context Protocol](https://modelcontextprotocol.io) while it runs, so
agents such as Claude Code and Codex can read and design on the open canvas. Every tool call runs
through the editor like a user edit: one undo step per call, and the window repaints immediately.

* **Streamable HTTP** (built into the app): `http://127.0.0.1:29980/mcp`. Paper uses 29979, so
  both can run. Loopback only; requests from browsers (with an `Origin` header) and foreign
  `Host` headers are rejected. `SKETCH_RS_MCP_PORT=NNNN` changes the port; `SKETCH_RS_MCP=0`
  turns the server off.
* **stdio**: `sketch-rs-mcp` (built next to `sketch-rs`) relays stdio to the running app. It
  lists the tools even when the app is closed, and tells the agent to start Sketch if a call
  can't reach it.

### Claude Code

```sh
# HTTP (the app must be running)
claude mcp add --transport http --scope user sketch http://127.0.0.1:29980/mcp
# or stdio through the bridge
claude mcp add --scope user sketch -- /path/to/sketch-rs/target/release/sketch-rs-mcp
```

### Codex

```sh
# HTTP
codex mcp add sketch --url http://127.0.0.1:29980/mcp
# or stdio
codex mcp add sketch -- /path/to/sketch-rs/target/release/sketch-rs-mcp
```

which writes this to `~/.codex/config.toml` (either form works):

```toml
[mcp_servers.sketch]
url = "http://127.0.0.1:29980/mcp"
# command = "/path/to/sketch-rs/target/release/sketch-rs-mcp"
```

Then ask the agent something like *"In Sketch, create a 390×844 mobile artboard and design a
sign-in screen"*, or *"draw a flowchart of our deploy process on a new page, hand-drawn
style"*.

### Tools

Names and arguments follow Paper's MCP server where Sketch has an equivalent, so prompts
written for Paper work here too.

| Group | Tools |
|---|---|
| Projects & pages | `list_resources`, `open_file`, `create_file`, `create_page`, `rename_pages` |
| Read | `get_basic_info`, `get_selection`, `get_node_info`, `get_children`, `get_tree_summary`, `get_computed_styles`, `get_jsx`, `find_nodes`, `get_fill_image`, `get_font_family_info`, `get_guide` |
| See | `get_screenshot` (PNG image content of a node or the page), `export` (PNG/SVG/PDF files), `export_combined_pdf` |
| Write | `create_artboard`, `write_html`, `update_styles`, `set_text_content`, `rename_nodes`, `duplicate_nodes`, `move_nodes`, `delete_nodes`, `finish_working_on_nodes` (saves) |
| Sketch extras | `create_shape` (rectangles, ellipses, diamonds, stars, stickies, text, lines, arrows bound to shapes), `select_nodes`, `undo`, `redo` |

`write_html` takes HTML with inline styles, and [Taffy](https://github.com/DioxusLabs/taffy) lays
it out: flexbox, CSS grid, block flow with margin collapsing, percentages, min/max sizes,
aspect ratio, and absolute or relative positioning. Text gets size, weight (400, 500, 600, 700),
italic, color, line height, letter spacing, alignment, `text-transform` and rich-text runs
(`<b>`, `<i>`, `<span style>`). Boxes get backgrounds (colors, gradients, images), borders (also
per side), radii, `box-shadow` (also inset), opacity, blend modes, `overflow: hidden` with rounded
clipping, and `transform: translate()`. `<img>` takes local paths or `data:` URIs and `<svg>` is
imported as vectors. `update_styles` takes the same CSS as JSON, plus the hand-drawn keys
`sketch`, `roughness`, `fillStyle`, `strokeStyle`. Node ids look like `3fa9c1:14` (page and
node); `pageId` lets an agent work on another page without switching the user's view.

CSS that has no effect comes back in the result's `ignoredStyles`, so an agent can tell when
something didn't land. Everything measures as `box-sizing: border-box`, and elements have no
default margins.

## Tests

```sh
cargo test                                # unit, scenario, headless GPU, CLI, fidelity
SKETCH_RS_REQUIRE_GPU=1 cargo test         # fail (instead of skip) when no GPU adapter exists
cargo test --test fidelity -- --nocapture  # scores against Chromium; images in .artifacts/fidelity/
cargo test --release --test perf -- --nocapture   # 10k-element frame timings
```

* **Unit tests** cover geometry, vector paths, the rough.js port (compared with real
  rough.js 4.6.4 output), auto layout and constraints, booleans, rich text spans, the elbow
  router, formats and migrations, atomic writes, the project store, PDF structure, Excalidraw
  and SVG import, the palette's fuzzy matching and the shortcut table.
* **Scenario tests** drive the editor with synthetic pointer and keyboard events:
  `src/editor/tests.rs` (core editing), `tests/projects.rs` (projects, autosave, pages),
  `tests/figma.rs` (groups, components, auto layout, constraints, pen, booleans, rich text,
  guides), `tests/excalidraw.rs` (hand-drawn style, arrows, quick-connect, stickies, laser,
  library), `tests/workflow.rs` (layers drag, palette, shortcuts), `tests/io.rs` (export,
  import, clipboard, CLI), `tests/mcp.rs` (every MCP tool on a headless session, then an rmcp
  client over HTTP and through the `sketch-rs-mcp` stdio bridge).
* **Fidelity tests** (`tests/fidelity.rs`) send HTML through `write_html` and `get_screenshot`, and
  compare the result with a committed Chromium render. Each page has a budget of differing pixels
  that should only ever go down. Look at the side-by-sides too: a low score can still hide a
  missing badge or a wrong line break. `scripts/fidelity-refs.sh` re-renders the references.
* **Headless GPU tests** render through wgpu, assert pixels and write the screenshots below.
  One rasterizes the SVG export with resvg and compares it with the GPU render; one renders
  the PDF export with pdftoppm when installed.

| Milestone | Screenshot(s) in `docs/screenshots/` |
|---|---|
| 1. Local projects | `milestone1_home.png`, `milestone1_trash.png` |
| 2. Pages | `milestone2_pages.png` |
| 3. Figma core | `milestone3_figma_core.png`, `milestone3_canvas.png` |
| 4. Excalidraw core | `milestone4_whiteboard.png` |
| 5. Inspector, palette, shortcuts | `milestone5_inspector.png`, `milestone5_palette.png`, `milestone5_shortcuts.png` |
| 6. Import / export | `milestone6_pdf_page.png` (PDF rasterized by pdftoppm), `milestone6_excalidraw_import.png`, `milestone6_svg_import.png` |
| 7. MCP server | `milestone7_mcp.png` (a screen written through `write_html`, captured with `get_screenshot`), `milestone7_mcp_live_xvfb.png` (the live app after Claude Code drew into it over the stdio bridge) |
| Sketch 0.1 | `m1_*` … `m6_*`, `app_demo.png`, `x11_window_xvfb.png` |

## Using it

![Whiteboard features](docs/screenshots/milestone4_whiteboard.png)

The tools sit in the floating bar at the bottom of the canvas. Select a layer to edit it in
the **Design** panel on the right. The **Layers** panel on the left shows the tree: click to
select, Shift-click to add, double-click to rename, and use 👁 / 🔒 to hide or lock. The
**Up/Down** buttons reorder.

* Draw a frame around existing shapes and it adopts them. Shapes drawn inside a frame become
  its children, and dragging a layer into or out of a frame re-parents it.
* Top-level frames that contain content act as canvas regions. Drag from their body to
  marquee-select inside them, and drag the frame by its name label.
* The pencil keeps a constant on-screen width at any zoom. With nothing selected, set width,
  smoothing and color in the Design panel.
* Hold ⌘/Ctrl while dragging to suspend snapping.

### Keyboard shortcuts

⌘ on macOS, Ctrl on Linux and Windows. Press `?` or ⌘/ in the app for the full grouped list
(100+ bindings), or ⌘K to search every command.

| Keys | Action | Keys | Action |
|---|---|---|---|
| V / 1 | Select | ⌘K | Command palette |
| H, or hold Space | Hand (pan) | ⌘Z / ⌘⇧Z | Undo / redo |
| F or A | Frame | ⌘G / ⌘⇧G | Group / ungroup |
| R / 2, O / 4, D / 3 | Rectangle, ellipse, diamond | ⌘⌥G | Frame selection |
| ⇧Y, ⇧S | Polygon, star | ⌘⌥K / ⌘⌥B | Create component / detach |
| L / 6, ⇧L / 5 | Line, arrow | ⇧A / ⌥⇧A | Add / remove auto layout |
| P, ⇧P / 7 | Pen, pencil | ⌥A ⌥H ⌥D / ⌥W ⌥V ⌥S | Align left/center/right, top/middle/bottom |
| T / 8, S | Text, sticky note | ⌥⇧H / ⌥⇧V | Distribute spacing |
| K | Laser pointer | ⌥⇧U S I E | Union, subtract, intersect, exclude |
| E / 0 | Eraser | ⌘E | Flatten |
| 9, ⌘⇧K | Place image | ⌘B / ⌘I / ⌘U / ⌘⇧X | Bold, italic, underline, strikethrough |
| Enter / ⇧Enter | Edit (text, path) / select parent | ⌘⌥H | Hand-drawn style |
| Tab / ⇧Tab | Next / previous sibling | ⇧H / ⇧V | Flip |
| ⌥-drag | Duplicate while dragging | ⌘⌥C / ⌘⌥V | Copy / paste properties |
| ⌥ hover | Measure distances | ⌘R | Rename |
| ⌘= / ⌘- / ⇧0 | Zoom in / out / 100% | ⇧1 / ⇧2 | Zoom to fit / selection |
| PageUp / PageDown | Previous / next page | ⌘\\ | Show / hide UI |

## Feature parity: Paper / Figma / Excalidraw vs Sketch

Columns 2–4 say whether each product has the feature (✓), per upstream's research: Figma from
public docs (`analysis/FIGMA_NOTES.md`), Excalidraw from its MIT source
(`analysis/EXCALIDRAW_NOTES.md`). The last column is Sketch: ✅ done, ⚠️ partial (see
Known gaps), ⏳ planned in a later milestone, ❌ not planned.

| Feature | Paper | Figma | Excalidraw | Sketch |
|---|---|---|---|---|
| Local projects, recents grid, trash | (cloud dashboard) | ✓ (cloud) | (single file) | ✅ Folders in `~/Documents/sketch-rs`, thumbnails, rename/duplicate/trash/restore |
| Autosave, crash-safe, versioned files | ✓ (cloud) | ✓ (cloud) | ✓ (browser storage) | ✅ 1 s autosave, atomic writes, migration chain (page v1→v3) |
| Pages, infinite canvas per page | ✓ | ✓ | – | ✅ Per-page history and camera, reorder/duplicate/delete |
| Frames with clipping | ✓ | ✓ | ✓ | ✅ Rounded corners clip; ⚠️ only the innermost rounded frame rounds, outer ones clip to their box |
| Groups | ✓ | ✓ | ✓ | ✅ Bounds follow children, click selects group, double-click enters, scale on resize |
| Components, instances, overrides | – | ✓ | (library only) | ✅ Live sync, per-property overrides, reset, detach, go to main |
| Auto layout (direction, gap, padding, hug/fill/fixed, wrap) | ✓ (flexbox) | ✓ | – | ✅ Taffy flexbox, grid and block flow; drag-reorder in flow, absolute children; ⚠️ grid and block have no inspector controls yet |
| Box shadows, per-side borders | ✓ | ✓ | – | ✅ Outer and inset shadows (exact in SVG, banded in PDF); ⚠️ set through CSS, no inspector controls yet |
| Constraints | – | ✓ | – | ✅ Left/right/stretch/center/scale per axis, ⌘ to ignore |
| Pen tool, vector point editing | ✓ | ✓ | – | ✅ Corner/smooth points, handles (⌥ breaks), close, delete, toggle |
| Boolean ops (union/subtract/intersect/exclude), flatten | (roadmap) | ✓ | – | ✅ ⚠️ destructive (result is a path, curves flattened) |
| Gradients (linear, radial) | ✓ | ✓ | – | ✅ Multi-stop, angle, radius; ⚠️ no on-canvas gradient handles |
| Image fills | ✓ | ✓ | ✓ (image element) | ✅ Fill/fit/stretch/tile, place image (⌘⇧K), drag-and-drop, stored in project |
| Strokes: width, align, dash, caps, joins | ✓ | ✓ | ✓ | ✅ |
| Corner radius (per corner), polygons, stars | ✓ | ✓ | – | ✅ |
| Opacity, blend modes | ✓ | ✓ | ✓ (opacity) | ✅ ⚠️ canvas renders 5 modes exactly, all 16 exact in SVG |
| Rich text (per-range weight/italic/size/colour) | ✓ | ✓ | – | ✅ Weights 400–700, underline, strikethrough, line height, letter spacing; ⚠️ the inspector only toggles bold |
| Align / distribute | ✓ | ✓ | ✓ | ✅ ⌥A/D/W/S/H/V, ⌥⇧H/V |
| Smart guides, spacing, measurement | ✓ | ✓ | – | ✅ Edge/center snap, equal spacing, ⌥-hover distances |
| Hand-drawn (rough) style per element | – | – | ✓ | ✅ rough.js 4.6.4 port, verified against rough.js output; sloppiness, fill styles |
| Bound arrows, elbow arrows, quick shapes, sticky notes | – | (FigJam) | ✓ | ✅ Orbit binding that follows shapes, straight/curved/elbow, 5 heads, diamond, quick-connect, stickies |
| Laser pointer, shape library | – | (FigJam) | ✓ | ✅ Fading laser trail; shared library with starter items |
| Inspector, layers drag-reorder, command palette (⌘K) | ✓ | ✓ | ✓ | ✅ Sectioned inspector, tree drag above/below/into, fuzzy palette over actions, pages, layers, library |
| Export PNG / SVG / PDF of selection, frame, page | ✓ | ✓ | ✓ (PNG/SVG) | ✅ Selection, each frame, page or all pages; vector PDF with gradients, images, blend modes; copy as PNG/SVG |
| Import .excalidraw / SVG, copy-paste across projects | – | ✓ (SVG) | ✓ | ✅ .excalidraw scenes, .excalidrawlib libraries, SVG (paths, gradients, images, text), system clipboard between pages, projects and apps |
| Freehand ink with pressure | – | (pencil) | ✓ | ✅ Smoothing, pressure, eraser |
| MCP server for AI agents | ✓ | ✓ (Dev Mode) | ✓ (community) | ✅ Streamable HTTP in the app + stdio bridge; `write_html`, screenshots, whiteboard shapes |
| Multiplayer, cloud sync, plugins | ✓ | ✓ | ✓ | ❌ local-first by design |

## Known gaps

* **The document model isn't HTML.** Nodes are geometric, and CSS maps onto them. Design tokens
  and CSS variables are missing, and `get_jsx` writes inline styles only (`tailwind` is accepted
  and ignored), with text as spans rather than the original tags.
* **Fonts.** Text is Inter only, with no kerning, ligatures or complex scripts because there is no
  shaping, and no font picker. Any other `font-family` is reported in `ignoredStyles`. Inter's
  Medium and SemiBold are bundled upright only, so italics use the nearest italic.
* **CSS not supported** (all reported in `ignoredStyles`): rotate and scale transforms, percentage
  translates, `z-index`, `content-box`, `flex-direction: *-reverse`, dashed borders on a single
  side, filters, tables, `calc()`, `vw`/`vh`. Absolute children offset from their parent even if
  it isn't `position: relative`. Inline elements can't have a background or padding unless they
  are `inline-block`. A smaller inline run doesn't grow its line box the way a browser's inherited
  `line-height` does, which can make a line a few pixels shorter.
* **Rendering approximations.** Group opacity multiplies into children instead of compositing the
  group. Overlay, color dodge/burn, hard/soft light, difference, exclusion, hue, saturation,
  color and luminosity blend as Normal on the canvas (they are exact in SVG export). Only the
  innermost rounded clipping frame rounds its corners.
* **Booleans are destructive**: the result is a new vector path with flattened curves.
  Gradients are edited in the inspector only.
* **macOS hand testing is missing.** On a clean arm64 macOS 26 runner, this fork builds, passes
  the full test suite, opens its window on Metal, connects to Claude Code and takes designs over
  MCP with the same fidelity scores as Linux. Nobody has yet tried mouse, trackpad and keyboard
  input or the menus by hand, and Intel Macs are untested. On Linux, the real window was exercised
  under Xvfb; the screenshot is `x11_window_xvfb.png`.
* **Pen pressure.** winit 0.30 does not expose tablet pressure for pen events that arrive as
  mouse input, which is common with macOS and Wacom tablets. Pressure comes from touch/pen
  `force` when the platform reports it. Otherwise it is simulated from stroke velocity.
* **Text input.** IME preedit is not displayed (committed text works).
* **No rotation.** Nodes are axis-aligned boxes; rotated imports are baked into paths.
* **Undo granularity.** Document renames from the Design panel are not undoable, and the
  selection is not part of history.
* **Performance.** Nodes are tessellated once per zoom octave and cached; unchanged frames
  reuse the previous item list and GPU buffers, so panning uploads nothing. With 10k visible
  nodes a cached frame takes ~3 ms (release, `tests/perf.rs`); the first frame at a new zoom
  octave takes ~140 ms on a CPU. Not tuned for 100k+.
* **PDF**: text is outlined (not selectable); gradient stop transparency uses the first
  stop's alpha; tiled image fills are drawn as repeated images.
* **Import**: Excalidraw rotation is baked into paths (text rotation is dropped), bound text
  inside shapes stays a separate text layer (except sticky notes), and fonts map to Inter.
  SVG patterns, filters, masks and clip paths are not imported.
* **MCP**: one project is open at a time, so `fileId` must name the open project (call
  `open_file` to switch; the user's window switches too). No design tokens, comments or AI
  image generation. Moving or duplicating layers between pages isn't supported. Remote
  `http(s)` images aren't fetched; pass a local path or a `data:` URI.
* **Linux file dialogs** need an XDG desktop portal. Without one, Save/Open dialogs return
  nothing. The headless CLI is unaffected.

## Project layout

```
src/geom.rs        vectors, rects, camera
src/vector.rs      bezier paths: flatten, hit test, SVG data, shapes, dashes
src/model/         Node / NodeKind / paints / Document arena
src/history.rs     transactions, undo/redo
src/layout.rs      layout (Taffy), constraints, group bounds
src/ink.rs         stroke smoothing, pressure, outline
src/text.rs        rich text layout and glyph outlines
src/assets.rs      image registry (content-hashed)
src/project/       project store, manifest/page formats, migrations
src/session.rs     open project, autosave, thumbnails
src/editor/        tools, structure (groups/components), vector ops, pages, text editing
src/render/        draw ops → cached meshes → wgpu; headless capture
src/io/            PNG, SVG, PDF, export scopes, Excalidraw + SVG import, clipboard,
                   atomic writes
src/connector.rs   arrow bindings, elbow router, arrowheads
src/library.rs     shared shape library
src/ui/            home screen, pages, layers, inspector
src/app.rs         winit application
src/mcp/           MCP server (rmcp): tools, HTML/CSS mapping, HTTP service
src/bin/sketch-rs-mcp.rs  stdio bridge to the running app
analysis/          behavior notes on Figma and Excalidraw
docs/screenshots/  headless renders produced by `cargo test`
scripts/           fidelity-refs.sh: re-renders the Chromium references
tests/fidelity/    pages and Chromium references for tests/fidelity.rs
```

## License

MIT OR Apache-2.0 for the code in this repository. Inter (Regular, Medium, SemiBold) is
© The Inter Project Authors under the SIL Open Font License 1.1; see `assets/fonts/OFL.txt`.
Ported code and dependencies keep their own licenses; see `THIRD_PARTY.md`.
