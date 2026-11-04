# Excalidraw porting notes (for Sketch)

Source studied:

- **Excalidraw** monorepo at `~/re/excalidraw`, commit `e65eca999e`
  ("refactor(editor): app.selection, and the rest of the selection tool out of App (#12258)").
  All `packages/...` paths below are relative to `~/re/excalidraw`.
- **roughjs 4.6.4**: the exact version pinned by Excalidraw (`packages/excalidraw/package.json:114`,
  `packages/utils/package.json:59`). Fetched read-only with `npm pack` into `~/re/roughjs/v464/x_roughjs-4.6.4.tgz/package/bin/`.
  roughjs 4.6.6 is at `~/re/roughjs/package/bin/`; the differences are listed in §2.9.
- roughjs dependencies, also fetched read-only into `~/re/roughjs/v464/`:
  `hachure-fill@0.5.2`, `points-on-curve@0.2.0` (roughjs dep; Excalidraw itself imports `points-on-curve@1.0.1`, whose algorithms are identical),
  `points-on-path@0.2.1`, `path-data-parser@0.1.0`.

Line references use the form `file:line` and are from those exact sources.

## Licenses (must be honoured)

Excalidraw (`~/re/excalidraw/LICENSE`, MIT):

> MIT License
>
> Copyright (c) 2020 Excalidraw

`packages/laser-pointer/LICENSE` (MIT): `Copyright (c) 2023 Excalidraw`

roughjs (`roughjs-4.6.4/package/LICENSE`, MIT):

> MIT License
>
> Copyright (c) 2019 Preet Shihn

Transitive roughjs deps (all MIT, same author): hachure-fill `Copyright (c) 2023 Preet Shihn`;
points-on-curve, points-on-path and path-data-parser `Copyright (c) 2020 Preet Shihn`.
`packages/fractional-indexing` is vendored CC0 (no attribution required, but credit it anyway).
Excalidraw's freedraw uses `perfect-freehand@1.2.0` (MIT, Steve Ruiz). It is not studied here, only referenced.

**Rule for Sketch:** any code ported from these sources (algorithms transcribed line by line, constants tables,
the RNG, the hachure scanline, the A* router, and so on) must be credited in `THIRD_PARTY.md` with the project
name, the copyright line above and the full MIT text. Also put a short `// Ported from <project> (<path>), MIT` comment
at the top of each ported Rust module.

---

## 1. Element model and file formats

### 1.1 Common element fields (`packages/element/src/types.ts:40-87`)

Every element has all of these keys (JSON camelCase):

| key | type | notes |
|---|---|---|
| `id` | string | nanoid (21 chars). Duplicate ids on load get a new `randomId()` (`restore.ts:1000`) |
| `type` | string | `rectangle, diamond, ellipse, text, line, arrow, freedraw, image, frame, magicframe, embeddable, iframe, stickynote` (+ legacy `selection`, `draw`) |
| `x`,`y` | f64 | scene coordinates of the **unrotated** top-left of the element box. For linear/freedraw elements this is the origin that `points` are relative to |
| `width`,`height` | f64 | unrotated box size. For linear elements it is derived from the points bbox (`getSizeFromPoints`) |
| `angle` | f64 radians | rotation about the box centre `(x+w/2, y+h/2)`, or the points-bbox centre for linear elements; clockwise in screen space (y down) |
| `strokeColor` | CSS color | default `"#1e1e1e"` |
| `backgroundColor` | CSS color | default `"transparent"` |
| `fillStyle` | `"hachure" \| "cross-hatch" \| "solid" \| "zigzag"` | default `"solid"` (`constants.ts:517`) |
| `strokeWidth` | f64 | thin 1, medium 2 (default), bold 4. freedraw uses 0.5 / 1 / 2 (`constants.ts:487-506`) |
| `strokeStyle` | `"solid" \| "dashed" \| "dotted"` | |
| `roughness` | f64 | 0 architect, 1 artist (default), 2 cartoonist |
| `opacity` | 0..100 | integer percent |
| `roundness` | `null \| {type: 1\|2\|3, value?: f64}` | 1 LEGACY, 2 PROPORTIONAL_RADIUS, 3 ADAPTIVE_RADIUS (`constants.ts:450`) |
| `seed` | int | roughjs seed, `floor(rand*2^31)` (`common/src/random.ts`) |
| `version` | int | incremented on each mutation (`mutateElement.ts:142`) |
| `versionNonce` | int | random on each mutation. Ties resolve to the **lower** nonce (`data/reconcile.ts:23-44`) |
| `index` | string \| null | fractional index (z-order), see §8.1 |
| `isDeleted` | bool | tombstone. Keep deleted elements in memory, skip them when rendering |
| `groupIds` | string[] | **innermost first, outermost last** ("Ordered from deepest to shallowest", `types.ts:71-73`) |
| `frameId` | string \| null | containing frame |
| `boundElements` | `null \| {id, type:"arrow"\|"text"}[]` | back-references: arrows bound to me, or my label |
| `updated` | epoch ms | |
| `created` | epoch ms \| null | new at this commit; absent in older files |
| `link` | string \| null | hyperlink |
| `locked` | bool | |
| `customData` | object? | optional; preserve it |

Type-specific fields:

- **text** (`types.ts:253-291`): `text` (the *wrapped* text with `\n` inserted), `originalText` (the unwrapped source),
  `fontSize`, `fontFamily` (number: 1 Virgil, 2 Helvetica, 3 Cascadia, 5 Excalifont (the default), 6 Nunito,
  7 Lilita One, 8 Comic Shanns, 9 Liberation Sans, 10 Assistant; `constants.ts:143`),
  `textAlign` `left|center|right`, `verticalAlign` `top|middle|bottom`, `containerId`, `autoResize` (default true),
  `lineHeight` (unitless; 1.25 for most fonts, 1.15 Helvetica/Lilita/Liberation, 1.2 Cascadia; `common/src/font-metadata.ts`),
  `labelPosition?` (0..1 arc-length position of an arrow label), and `baseFontSize` (sticky notes only, else null).
- **line / arrow** (`types.ts:369-421`): `points: [[x,y],...]` local to `(x,y)`. `points[0]` is normally `[0,0]`,
  but don't rely on it for arrows (restore normalises it). Also `startBinding`, `endBinding`, `startArrowhead`, `endArrowhead`.
  line adds `polygon: bool` (closed and fillable).
  arrow adds `elbowed: bool`. Elbow arrows add `fixedSegments: {start,end,index}[] | null`, `startIsSpecial`, `endIsSpecial`.
  Older files may also have `lastCommittedPoint` (ignore it).
- **Arrowheads**: `null | "arrow" | "bar" | "circle" | "circle_outline" | "triangle" | "triangle_outline" | "diamond" | "diamond_outline" |
  "cardinality_one" | "cardinality_many" | "cardinality_one_or_many" | "cardinality_exactly_one" | "cardinality_zero_or_one" | "cardinality_zero_or_many"`.
  Legacy aliases: `dot` maps to `circle` and `crowfoot_*` maps to `cardinality_*` (`arrowheads.ts:3-21`).
- **Binding (current schema v2)** (`types.ts:316-333`): `{ elementId, fixedPoint: [fx, fy], mode: "inside" | "orbit" | "skip" }`.
  `fixedPoint` is a ratio of the target's unrotated box: local point = `(x + w*fx, y + h*fy)`, then rotated by the target's angle.
  **Legacy v1 schema**: `{ elementId, focus: f64 (-1..1), gap: f64, fixedPoint?: null|[..] }`, with no `mode`.
  restore migrates it (§8.4).
- **freedraw** (`types.ts:430-437`): `points` (local), `pressures: f64[]` (same length, or empty), `simulatePressure: bool`,
  `strokeOptions: {variability: "variable"|"constant", streamline: f64 (default 0.5)}`.
- **image** (`types.ts:161-171`): `fileId` (normally the 40-hex SHA-1 of the file bytes, `data/blob.ts:261`),
  `status: "pending"|"saved"|"error"`, `scale: [sx, sy]` (±1 for flips), `crop: null | {x,y,width,height,naturalWidth,naturalHeight}`.
- **frame / magicframe**: `name: string|null`.
- **stickynote** (new type at this commit): `baseHeight`. See §7.4.
- **embeddable / iframe**: no extra fields. The URL is in `link`.

### 1.2 `.excalidraw` file (`packages/excalidraw/data/json.ts:52-75`)

```jsonc
{
  "type": "excalidraw",            // EXPORT_DATA_TYPES.excalidraw (constants.ts:347)
  "version": 2,                    // VERSIONS.excalidraw (constants.ts:419)
  "source": "https://excalidraw.com",
  "elements": [ /* ExcalidrawElement[], array order == z-order (back to front), deleted ones included */ ],
  "appState": { "gridSize": 20, "gridStep": 5, "gridModeEnabled": false,
                "viewBackgroundColor": "#ffffff", "lockedMultiSelections": {} },
  "files": { "<fileId>": { "mimeType": "image/png", "id": "<fileId>",
             "dataURL": "data:image/png;base64,...", "created": 1700000000000,
             "lastRetrieved": 1700000000000, "version"?: 1 } }
}
```

- Validity check: `type === "excalidraw"`, and `elements` is missing or an array, and `appState` is missing or an object (`json.ts:115-126`).
- The exported appState keys are the ones flagged `export: true` in `packages/excalidraw/appState.ts:150-290`:
  `gridSize, gridStep, gridModeEnabled, viewBackgroundColor, lockedMultiSelections`. Older files carry many more keys. Ignore unknown keys.
- `files` only holds files referenced by non-deleted image elements (`filterOutDeletedFiles`, `json.ts:34-50`).
- MIME type `application/vnd.excalidraw+json`. Images and SVGs with an embedded scene use `image/png` and `image/svg+xml` (`constants.ts:311-333`).

### 1.3 Clipboard (`packages/excalidraw/clipboard.ts:143-193`)

```json
{ "type": "excalidraw/clipboard", "elements": [...], "files": { "<fileId>": BinaryFileData } }
```

- It is written as text/plain JSON (and as MIME `application/vnd.excalidraw.clipboard+json` where possible).
  `files` is omitted when not provided.
- Elements whose containing frame is *not* also copied get `frameId: null`.
- The API variant uses `type: "excalidraw-api/clipboard"`.
- On paste, the element JSON goes through `restoreElements` and then `duplicateAtSceneCoords` (§6.2).

### 1.4 Importer mapping (geometry per type)

Let `R(p)` rotate point `p` by `angle` about the centre `c`. Here `c = (x + w/2, y + h/2)` for box elements.
For linear and freedraw elements `c = (x + (minX+maxX)/2, y + (minY+maxY)/2)` over the points (or over the curve bounds when `roundness` is set),
per `LinearElementEditor.getElementAbsoluteCoords` and `bounds.ts:246-287`.

| type | geometry |
|---|---|
| rectangle / embeddable / iframe | box `(x,y,w,h)`; rounded when `roundness` is set (radius in §2.2) |
| diamond | 4 vertices from `getDiamondPoints` (§7.2), local to `(x,y)` |
| ellipse | centre `(x+w/2, y+h/2)`, radii `w/2, h/2` |
| line / arrow | polyline `(x+px, y+py)` for each point; `roundness != null` means Catmull-Rom curve through the points (roughjs `curve`); `elbowed` means an orthogonal polyline drawn with corner radius 16 (§2.2) |
| freedraw | points local to `(x,y)`; the stroke is a perfect-freehand outline (§2.8) |
| text | box `(x,y,w,h)`; lines are `text.split("\n")`, line height in px = `fontSize*lineHeight`; baseline per §7.3. If `containerId` is set, the text sits inside that container (positions are already baked into x/y, but recompute after a container edit using §7.3) |
| image | `files[fileId].dataURL` (base64 data URL) drawn into `(x,y,w,h)`; apply `scale` flips and `crop` (source rect in natural pixels) |
| frame | box; children are elements with `frameId == frame.id`, clipped to the frame box; name label drawn above (14px, `#999999`, `FRAME_STYLE`, `constants.ts:209-223`) |
| stickynote | box filled with `backgroundColor` (default `#ffdf6b`), label via the bound text (§7.4) |

Z-order is the array order (§8.1 for `index`). Bound text is placed directly after its container (`repairBoundTextElementOrder`, `restore.ts:849`).
Frame children sit before (below) the frame element (`frame.ts:521-536`).

Suggested Rust shape:

```rust
#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
struct ExFile { r#type: String, version: Option<u32>, source: Option<String>,
                elements: Option<Vec<ExElement>>, app_state: Option<serde_json::Value>,
                files: Option<HashMap<String, ExBinaryFile>> }

#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
struct ExElement {
    id: String, #[serde(rename = "type")] kind: String,
    #[serde(default)] x: f64, #[serde(default)] y: f64,
    #[serde(default)] width: f64, #[serde(default)] height: f64, #[serde(default)] angle: f64,
    stroke_color: Option<String>, background_color: Option<String>, fill_style: Option<String>,
    stroke_width: Option<f64>, stroke_style: Option<String>, roughness: Option<f64>, opacity: Option<f64>,
    seed: Option<i64>, version: Option<i64>, version_nonce: Option<i64>, index: Option<String>,
    #[serde(default)] is_deleted: bool, #[serde(default)] group_ids: Vec<String>, frame_id: Option<String>,
    bound_elements: Option<Vec<BoundRef>>, roundness: Option<Roundness>, link: Option<String>,
    #[serde(default)] locked: bool,
    // type specific: keep as Option or use #[serde(flatten)] extra: Map<String, Value>
    points: Option<Vec<[f64; 2]>>, pressures: Option<Vec<f64>>, simulate_pressure: Option<bool>,
    start_binding: Option<ExBinding>, end_binding: Option<ExBinding>,
    start_arrowhead: Option<String>, end_arrowhead: Option<String>, elbowed: Option<bool>,
    text: Option<String>, original_text: Option<String>, font_size: Option<f64>, font_family: Option<u32>,
    text_align: Option<String>, vertical_align: Option<String>, container_id: Option<String>,
    line_height: Option<f64>, file_id: Option<String>, scale: Option<[f64; 2]>, crop: Option<ExCrop>,
    name: Option<String>,
    #[serde(flatten)] extra: serde_json::Map<String, serde_json::Value>, // forward-compat, like restore.ts:503
}
```

Apply the defaults from §8.4 after deserialising (they mirror `restoreElementWithProperties`).

---

## 2. Hand-drawn rendering

### 2.1 Excalidraw element → roughjs `Options` (`packages/element/src/shape.ts:168-260`)

```
dash(dashed)  = [8, 8 + strokeWidth]            // shape.ts:168
dash(dotted)  = [1.5, 6 + strokeWidth]          // shape.ts:170

options = {
  seed: element.seed,
  strokeLineDash: dashed ? [8, 8+sw] : dotted ? [1.5, 6+sw] : None,
  disableMultiStroke: strokeStyle != "solid",
  strokeWidth: strokeStyle != "solid" ? sw + 0.5 : sw,
  fillWeight: sw / 2,                       // always from the element sw, not the +0.5 one
  hachureGap: sw * 4,
  roughness: adjustRoughness(element),
  stroke: strokeColor,
  preserveVertices: continuousPath || element.roughness < 2,   // ROUGHNESS.cartoonist == 2
}
rectangle|iframe|embeddable|diamond|ellipse: fillStyle = el.fillStyle; fill = transparent? None : bg;
                                             ellipse: curveFitting = 1
line|freedraw: only if isPathALoop(points): fillStyle + fill (bg, or None when "transparent")
arrow: no fill
```

`continuousPath` is `true` only for the rounded-rect path, the rounded-diamond path and elbow arrows.

`adjustRoughness` (`shape.ts:172-193`):

```
max = max(w,h); min = min(w,h)
keep el.roughness if (min >= 20 && max >= 50)
                 || (min >= 15 && roundness != null && canChangeRoundness(type))   // rect, iframe, embeddable, line, diamond, stickynote, image
                 || (isLinear && max >= 50)
else return min(roughness / (max < 10 ? 3 : 2), 2.5)
```

`isPathALoop(points)` (`element/src/utils.ts:512-526`): `points.len() >= 3 && dist(first, last) <= 8 / zoom` (`LINE_CONFIRM_THRESHOLD = 8`).

Unspecified options take the roughjs defaults (`generator.js:9-30`):
`maxRandomnessOffset 2, roughness 1, bowing 1, strokeWidth 1, curveTightness 0, curveFitting 0.95, curveStepCount 9,
fillStyle "hachure", fillWeight -1, hachureAngle -41, hachureGap -1, dashOffset -1, dashGap -1, zigzagOffset -1, seed 0,
disableMultiStroke false, disableMultiStrokeFill false, preserveVertices false, fillShapeRoughnessGain 0.8`.

### 2.2 Shape generation per element (`shape.ts:770-1016`)

Shapes are generated in **element-local, unrotated** coordinates. The renderer translates by `(x,y)` and rotates about the centre.

- **rectangle/iframe/embeddable** (`shape.ts:787-831`):
  - `roundness` set: `r = getCornerRadius(min(w,h), el)`, then
    `generator.path("M r 0 L w-r 0 Q w 0, w r L w h-r Q w h, w-r h L r h Q 0 h, 0 h-r L 0 r Q 0 0, r 0", opts(continuous=true))`.
  - otherwise `generator.rectangle(0, 0, w, h, opts)`.
  - `getCornerRadius(x, el)` (`utils.ts:528-547`): PROPORTIONAL/LEGACY gives `x*0.25`. ADAPTIVE: `fixed = roundness.value ?? 32`;
    returns `x*0.25` if `x <= fixed/0.25`, otherwise `fixed`. No roundness gives 0.
- **diamond** (`shape.ts:833-878`): points `[top, right, bottom, left]` from §7.2.
  - Unrounded: `generator.polygon(points)`.
  - Rounded: `vr = getCornerRadius(|topX-leftX|)`, `hr = getCornerRadius(|rightY-topY|)`, then path
    `M topX+vr topY+hr L rightX-vr rightY-hr C rightX rightY, rightX rightY, rightX-vr rightY+hr L bottomX+vr bottomY-hr C bottomX bottomY, bottomX bottomY, bottomX-vr bottomY-hr L leftX+vr leftY+hr C leftX leftY, leftX leftY, leftX+vr leftY-hr L topX-vr topY+hr C topX topY, topX topY, topX+vr topY+hr`
    with `continuous=true`.
- **ellipse** (`shape.ts:880-889`): `generator.ellipse(w/2, h/2, w, h, opts)` (centre form).
- **line/arrow** (`shape.ts:890-985`):
  - elbowed: `generator.path(generateElbowArrowShape(points, 16), opts(continuous=true))`.
  - `!roundness`: `polygon(points)` if `opts.fill`, otherwise `linearPath(points)`.
  - `roundness`: `curve(points)`.
  - Arrowheads are then appended as extra drawables (§2.7).
  - `generateElbowArrowShape` (`shape.ts:1018-1085`): for each interior point i, `corner = min(16, |p_i p_{i+1}|/2, |p_i p_{i-1}|/2)`.
    Emit `L (p_i - corner along incoming dir)`, then `Q p_i, (p_i + corner along outgoing dir)`. Start with `M p0` and end with `L p_last`.
- **freedraw** (`shape.ts:986-1005`): (1) if it is a loop, a fill `generator.curve(simplify(points, 0.75), {...opts, stroke:"none"})`;
  (2) a filled SVG path of the perfect-freehand outline in `strokeColor` (`renderElement.ts:496-515`).
- **frame, text, image, stickynote**: not drawn by roughjs.

Canvas drawing of a roughjs `Drawable` (`roughjs/bin/canvas.js`): each `set` is one of
`path` (stroke with `strokeWidth` and dash), `fillPath` (fill; the rule is `evenodd` for shape `curve|polygon|path`, otherwise `nonzero`)
or `fillSketch` (stroke with `fillWeight`, or `strokeWidth/2` if that is negative, in the **fill** colour).
Ops are `move` → moveTo, `bcurveTo` → cubic, `lineTo`. Each set gets its own beginPath + stroke/fill.

### 2.3 roughjs RNG (`roughjs/bin/math.js`, `renderer.js:206-216`)

```js
next() { if (this.seed) return ((2**31 - 1) & (this.seed = Math.imul(48271, this.seed))) / 2**31;
         else return Math.random(); }
```

- One `Random` is created lazily per merged options object: `o.randomizer ||= new Random(o.seed || 0)`. Seed 0 means non-deterministic.
- Excalidraw's restore defaults `seed ?? 1`, so seed 0 should be treated as 1 in the port for determinism.
- **Every generator call** (`rectangle`, `ellipse`, `line`, …) merges `{...defaults, ...options}` into a *fresh* object, so each call restarts the RNG from `seed`.
- `cloneOptionsAlterSeed(o)` (`renderer.js:198-205`) copies o, clears `randomizer`, and sets `seed = seed + 1` (only if seed != 0).
- `Object.assign({}, o, {...})` copies done elsewhere (curve/path solid fill, cross-hatch, zigzag) **share** `o.randomizer`, so the sequence continues.

```rust
pub struct RoughRng { seed: i32 }       // seed = (js_seed as i64) as i32 (ToInt32 semantics)
impl RoughRng {
    pub fn next(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(48271);
        ((self.seed & 0x7fff_ffff) as f64) / 2147483648.0
    }
}
```

Never returns to 0 for a non-zero start, because 48271 is odd. When porting, keep the **exact call order**: JS evaluates arguments and array-literal elements left to right.

### 2.4 Offsets, lines, doubleLine (`renderer.js:212-299`)

```
_offset(min, max, o, gain=1)  = o.roughness * gain * (rng() * (max - min) + min)     // ALWAYS consumes one rng() even if roughness == 0
_offsetOpt(x, o, gain=1)      = _offset(-x, x, o, gain)

_doubleLine(x1,y1,x2,y2,o, filling=false):
   single = filling ? o.disableMultiStrokeFill : o.disableMultiStroke
   ops = _line(..., move=true, overlay=false)
   if !single: ops += _line(..., move=true, overlay=true)

_line(x1,y1,x2,y2,o,move,overlay):
   lenSq = (x1-x2)^2+(y1-y2)^2 ; len = sqrt(lenSq)
   gain  = len < 200 ? 1 : len > 500 ? 0.4 : -0.0016668*len + 1.233334
   offset = o.maxRandomnessOffset || 0          // 2
   if offset*offset*100 > lenSq: offset = len / 10
   half = offset / 2
   divergePoint = 0.2 + rng()*0.2                                  // rng #1
   midDispX = _offsetOpt(o.bowing*o.maxRandomnessOffset*(y2-y1)/200, o, gain)   // rng #2
   midDispY = _offsetOpt(o.bowing*o.maxRandomnessOffset*(x1-x2)/200, o, gain)   // rng #3
   R = overlay ? half : offset ; rnd() = _offsetOpt(R, o, gain)
   if move: push move(x1 + (pv?0:rnd()), y1 + (pv?0:rnd()))       // non-overlay uses offset, overlay uses half
   push bcurveTo(
       midDispX + x1 + (x2-x1)*divergePoint   + rnd(),
       midDispY + y1 + (y2-y1)*divergePoint   + rnd(),
       midDispX + x1 + 2*(x2-x1)*divergePoint + rnd(),
       midDispY + y1 + 2*(y2-y1)*divergePoint + rnd(),
       x2 + (pv?0:rnd()), y2 + (pv?0:rnd()))
   (pv = o.preserveVertices)
```

- `linearPath(points, close)` (`renderer.js:13-29`): 2 points gives `line`. More than 2: `_doubleLine` for each consecutive pair, plus last→first if `close`.
  0 or 1 points gives an empty path.
- `polygon(points) = linearPath(points, true)`.
- `rectangle(x,y,w,h) = polygon([(x,y),(x+w,y),(x+w,y+h),(x,y+h)])`.
- `line()` is a single `_doubleLine`.

### 2.5 Curves and ellipses (`renderer.js:42-77, 300-416`)

**curve(points)** (`renderer.js:42-49`):

```
o1 = _curveWithOffset(points, 1   * (1 + roughness*0.2),  o)
if !disableMultiStroke: o1 += _curveWithOffset(points, 1.5 * (1 + roughness*0.22), cloneOptionsAlterSeed(o))
```

`_curveWithOffset(points, off, o)` (`renderer.js:300-323`) builds
`ps = [p0+j, p0+j, p1+j, …, p_{n-1}+j, p_{n-1}+j]`, where each `j` is a fresh `(_offsetOpt(off), _offsetOpt(off))` (x then y).
It then calls `_curve(ps, null, o)`.

`_curve(points, closePoint, o)` (`renderer.js:324-359`). This is Catmull-Rom to Bezier with `s = 1 - curveTightness`:

```
len > 3: move(points[1]); for i in 1..(len-2) exclusive of i+2>=len:
            b1 = p[i]   + s*(p[i+1] - p[i-1]) / 6
            b2 = p[i+1] + s*(p[i]   - p[i+2]) / 6
            bcurveTo(b1, b2, p[i+1])
         (closePoint branch unused by Excalidraw)
len == 3: move(points[1]); bcurveTo(points[1], points[2], points[2])
len == 2: _doubleLine(points[0], points[1])
```

**ellipse(cx, cy, w, h)** (`renderer.js:50-77`):

```
generateEllipseParams:
   psq = sqrt(2π * sqrt(((w/2)^2 + (h/2)^2) / 2))
   stepCount = ceil(max(curveStepCount, curveStepCount / sqrt(200) * psq))      // curveStepCount = 9
   increment = 2π / stepCount
   rx = |w/2| + _offsetOpt(|w/2| * (1 - curveFitting))   // Excalidraw curveFitting = 1, so this adds 0 but STILL consumes rng
   ry = |h/2| + _offsetOpt(|h/2| * (1 - curveFitting))
ellipseWithParams:
   overlap = increment * _offset(0.1, _offset(0.4, 1, o), o)      // inner call evaluated first
   [ap1, cp1] = _computeEllipsePoints(increment, cx, cy, rx, ry, offset=1, overlap, o)
   ops = _curve(ap1)
   if !disableMultiStroke && roughness != 0:
       [ap2] = _computeEllipsePoints(increment, cx, cy, rx, ry, 1.5, 0, o);  ops += _curve(ap2)
   estimatedPoints = cp1          // used as the hachure fill polygon
```

`_computeEllipsePoints(inc, cx, cy, rx, ry, off, overlap, o)` (`renderer.js:360-416`):

```
if roughness == 0 (coreOnly):
   inc /= 4
   all.push(c + (rx cos(-inc), ry sin(-inc)))
   for a = 0; a <= 2π; a += inc: p = c + (rx cos a, ry sin a); core.push(p); all.push(p)
   all.push(c + (rx, 0)); all.push(c + (rx cos inc, ry sin inc))
else:
   rad = _offsetOpt(0.5) - π/2
   all.push((_offsetOpt(off) + cx + 0.9 rx cos(rad-inc), _offsetOpt(off) + cy + 0.9 ry sin(rad-inc)))
   end = 2π + rad - 0.01
   for a = rad; a < end; a += inc:
       p = (_offsetOpt(off) + cx + rx cos a, _offsetOpt(off) + cy + ry sin a); core.push(p); all.push(p)
   all.push(jitter + c + (rx cos(rad + 2π + overlap*0.5), ry sin(...)))
   all.push(jitter + c + (0.98 rx cos(rad + overlap),    0.98 ry sin(rad + overlap)))
   all.push(jitter + c + (0.9  rx cos(rad + overlap*0.5), 0.9  ry sin(rad + overlap*0.5)))
   (each jitter = (_offsetOpt(off), _offsetOpt(off)), x then y)
```

Generator-level ordering for `ellipse` (`generator.js:67-86`): params, then the stroke via `ellipseWithParams`, then the fill.
- `solid`: a **second** `ellipseWithParams` call (more rng, so a different wobble) with the type changed to `fillPath`.
- Otherwise: `patternFillPolygons([estimatedPoints])`.
- `paths = [fill, stroke]`.

Ordering for the others: `rectangle`, `polygon`, `curve` and `path` likewise compute the outline **first** (rng) and the fill **second**, but draw the fill first.

### 2.6 Fills

**solid polygon** (`renderer.js:143-158`): for each polygon with more than 2 points, `move(p0 + (_offsetOpt(2), _offsetOpt(2)))`,
then `lineTo(p_i + jitter)` for each point (2 rng per point, `maxRandomnessOffset` = 2). Type `fillPath`.

**solid curve/path** (`generator.js:117-139, 157-198`): re-run `curve()` / `svgPath()` with
`{...o, disableMultiStroke: true, roughness: roughness ? roughness + 0.8 : 0}` (shares the randomizer).
`_mergedShape` then drops every `move` except the first, giving one closed fill path.

**pattern fills** (`fillers/filler.js`):
- `zigzag` → ZigZagFiller
- `cross-hatch` → HatchFiller
- `dots`, `dashed`, `zigzag-line` (not used by Excalidraw)
- everything else → HachureFiller

**polygonHachureLines** (`fillers/scan-line-hachure.js`, 4.6.4):

```
angle = o.hachureAngle + 90                    // -41 + 90 = 49 degrees
gap   = o.hachureGap < 0 ? o.strokeWidth*4 : o.hachureGap ; gap = max(gap, 0.1)   // 4.6.6 also Math.round()s it
skip  = 1
if o.roughness >= 1 && (o.randomizer?.next() || Math.random()) > 0.7: skip = gap   // uses the SHARED randomizer (already advanced by the outline)
return hachureLines(polygons, gap, angle, skip)
```

**hachureLines** (`hachure-fill/bin/hachure.js:22-146`):

1. Rotate every polygon vertex **in place** about (0,0) by `+angle` degrees:
   `x' = x cos − y sin`, `y' = x sin + y cos`.
2. Run the straight horizontal scanline.
3. Rotate polygons and the output lines back by `−angle`.

Straight scanline:

```
for each polygon: close it (push first if last != first); keep only if > 2 vertices
edges = for each consecutive (p1,p2) with p1.y != p2.y:
          { ymin, ymax, x: x at ymin, islope: (p2.x-p1.x)/(p2.y-p1.y) }
sort edges by ymin, then x, then ymax
y = edges[0].ymin ; iteration = 0 ; active = []
while active or edges:
   move every edge with ymin <= y from edges into active (in order)
   active.retain(|e| e.ymax > y)
   active.sort_by(x)                                 // comparator returns 0 on ties (stable sort)
   if step != 1 || iteration % gap == 0:             // float modulo! with step==1 and integer gap -> every gap-th row
       for i in (0..active.len()).step_by(2): if i+1 < len:
           line [(round(active[i].x), y), (round(active[i+1].x), y)]
   y += step ; for e in active: e.x += step * e.islope ; iteration += 1
```

Watch out: with `step == 1` and a non-integer `gap`, `iteration % gap == 0` only fires on common multiples. Port the float `%` exactly (Rust `f64 %` matches JS).

**HachureFiller.renderLines** (`fillers/hachure-filler.js`): each line becomes `_doubleLine(..., filling=true)`.
`disableMultiStrokeFill` defaults false, so fill strokes are **doubled even for dashed/dotted elements**. Type `fillSketch`.

**cross-hatch** (`fillers/hatch-filler.js`): hachure at `hachureAngle`, then again with `hachureAngle + 90` (same randomizer); the ops are concatenated.

**zigzag** (`fillers/zigzag-filler.js`):

```
gap = max(hachureGap<0 ? sw*4 : hachureGap, 0.1)
lines = polygonHachureLines(polygons, {...o, hachureGap: gap})
a = hachureAngle in radians (-41 deg) ; dgx = gap*0.5*cos a ; dgy = gap*0.5*sin a
for [p1,p2] in lines where len > 0:
   push [(p1.x - dgx, p1.y + dgy), p2] and [(p1.x + dgx, p1.y - dgy), p2]
render each via _doubleLine(filling=true)
```

The fill polygon for each shape:
- rect: its 4 corners.
- polygon: the points.
- ellipse: `estimatedPoints`.
- curve (hachure): `pointsOnBezierCurves(curveToBezier(points), 10, (1+roughness)/2)`. Only when `points.len() >= 3` in 4.6.4.
- path: `pointsOnPath(d, 1, (1+roughness)/2)`.

### 2.7 Arrowheads (`shape.ts:295-577`, `bounds.ts:710-908`)

`getArrowheadPoints(el, shape, position, head, offsetMult=0)`:

- `ops = ` the first `path` set of `shape[0]`. Use op index `1` for "start" and `ops.len()-1` for "end"; it must be a `bcurveTo`.
- `p0` = the previous op's end point (or its move point). `p1, p2, p3` are this cubic's control and end points.
- `B(t) = (1-t)^3 p3 + 3t(1-t)^2 p2 + 3t^2(1-t) p1 + t^3 p0` (note: reversed). `(x1,y1) = B(0.3)`.
- `(x2,y2)` = p0 for "start", p3 for "end". `n = normalize((x2,y2) − (x1,y1))`.
- `size`: arrow 25; diamond/diamond_outline 12; cardinality many/one_or_many/zero_or_many 15 (`CROWFOOT_ARROWHEAD_SIZE`);
  cardinality one/exactly_one/zero_or_one 20 (`CARDINALITY_MARKER_SIZE`); default 15.
- `angle`: bar 90°, arrow 20°, else 25°.
- `length` = length of the last (or first) **raw** segment of `el.points`. `minSize = min(size, length * (diamond ? 0.25 : 0.5))`.
- `t = (x2,y2) − n*minSize*offsetMult` and `s = t − n*minSize`.
  - circle: `[tx, ty, diameter = |s−t| + strokeWidth − 2]`.
  - crowfoot many: rotate `t` about `s` by ∓angle, giving `[xs,ys,x3,y3,x4,y4]`.
  - otherwise rotate `s` about `t` by `−angle` and `+angle`, giving `[tx,ty,x3,y3,x4,y4]`.
  - diamond adds the opposite point `t − dir*2*minSize`.

Drawables:
- `arrow`/`bar`: two `line`s (x3,y3)→tip and (x4,y4)→tip, with `roughness = min(1, r)` and the dash removed (dotted uses `[1.5, 6+sw−1−1]`).
- `triangle(_outline)`: `polygon([tip, p3, p4, tip])`, solid fill (stroke colour, or the canvas background for the outline variant), `roughness = min(1, r)`.
- `circle(_outline)`: `circle`, solid fill, `roughness = min(0.5, r)`.
- `diamond(_outline)`: a 4-point polygon.
- Cardinality variants combine these with offsets `-0.25` / `-0.5` / `1.5` and circle scale `0.8` (`shape.ts:466-560`).

### 2.8 SVG paths: `generator.path(d)` (`generator.js:157-198`, `renderer.js:112-141, 440-464`)

1. `d = d.replace(/\n/g,' ').replace(/(-\s)/g,'-')`. The third `.replace('/(\s\s)/g',' ')` is a string literal, so it is a no-op bug.
2. `sets = pointsOnPath(d, tolerance 1, distance (1+roughness)/2)` (or `4−4*simplification` when `simplification < 1`; not used).
3. `shape = svgPath(d, o)`:
   - `segments = normalize(absolutize(parsePath(d)))` from path-data-parser. normalize turns H/V into L and S/T/Q/A into C.
     Q→C: `c1 = p0 + 2/3 (q − p0)`, `c2 = p + 2/3 (q − p)`.
   - `M`: set current and first (no op). `L`: `_doubleLine(current, p)`. `C`: `_bezierTo`. `Z`: `_doubleLine(current, first)`.
   - `_bezierTo(x1,y1,x2,y2,x,y,current,o)`:

     ```
     ros = [mro || 1, (mro || 1) + 0.3]        // [2, 2.3]
     for i in 0..(disableMultiStroke ? 1 : 2):
        i==0: move(current)    else: move(current + (pv?0:(_offsetOpt(ros[0]), _offsetOpt(ros[0]))))
        f = pv ? (x,y) : (x + _offsetOpt(ros[i]), y + _offsetOpt(ros[i]))      // f BEFORE control points
        bcurveTo(x1+_offsetOpt(ros[i]), y1+_offsetOpt(ros[i]), x2+_offsetOpt(ros[i]), y2+_offsetOpt(ros[i]), f)
     ```

4. Fill: solid with `sets.len()==1` re-runs svgPath with `roughness+0.8` and single stroke, then `_mergedShape`.
   Solid with several sets: `solidFillPolygon(sets)`. Otherwise `patternFillPolygons(sets)`.

**points-on-path** (`points-on-path/lib/index.js`): walks the normalized segments.
- `M` starts a new set.
- `L` pushes the point.
- `C` collects a pending cubic chain (`[last, c1, c2, p, c1, c2, p, ...]`) that is flattened by `pointsOnBezierCurves(chain, tolerance)` before the next L/M/Z or at the end.
- `Z` pushes the start point.
- Each set is then RDP-simplified with `distance`.

**points-on-curve** (`points-on-curve/lib/index.js`):

```
flatness(p0..p3): ux=(3p1.x-2p0.x-p3.x)^2, uy=(3p1.y-2p0.y-p3.y)^2, vx=(3p2.x-2p3.x-p0.x)^2, vy=(3p2.y-2p3.y-p0.y)^2
                  return max(ux,vx) + max(uy,vy)
split(points, off, tol, out):
   if flatness < tol: if out non-empty { if dist(out.last, p0) > 1 push p0 } else push p0 ; push p3
   else de Casteljau at t=0.5 and recurse on both halves
pointsOnBezierCurves(pts, tol=0.15, dist?): for each cubic (stride 3) split(); if dist>0: RDP(out, dist)
RDP(points, start, end, eps): farthest interior point from segment [s,e] by segment distance;
   if sqrt(maxDistSq) > eps recurse [start..maxNdx+1] and [maxNdx..end] else push s (if out empty) and e
curveToBezier(pts, tightness=0): 3 pts -> [p0,p1,p2,p2]; else duplicate first/last and emit [P0, (b1,b2,b3)...] with the _curve formula
simplify(points, d) = RDP over the whole list
```

**Freedraw stroke** (`shape.ts:1187-1312`): perfect-freehand `getStroke(points or [x,y,pressure], {size: sw*4.25, thinning: 0.6, smoothing: 0.5,
streamline: strokeOptions.streamline ?? 0.5, easing: t => sin(t*π/2), simulatePressure, last: true})`.
`strokeOptions.variability == "constant"` uses the LaserPointer outline (§5) with `size = sw*1.4`, `simplify 0` and pressure pinned to 1.
The outline becomes a path `M p0 Q p0 mid(p0,p1) p1 mid(p1,p2) … p_n mid(p_n,p0) L p0 Z` (values truncated to 2 decimals) and is filled with strokeColor.

### 2.9 roughjs 4.6.4 vs 4.6.6

- `scan-line-hachure.js:9`: 4.6.6 does `gap = Math.round(max(gap, 0.1))`.
- `generator.curve`: 4.6.6 accepts `Point[][]`, fills with `points.len() < 3` too, and handles 3-point curves specially.

Excalidraw ships **4.6.4**. Port 4.6.4.

Suggested Rust module layout:

```
rough/
  rng.rs        RoughRng
  options.rs    struct RoughOptions { max_randomness_offset, roughness, bowing, stroke_width, curve_tightness,
                curve_fitting, curve_step_count, fill_style, fill_weight, hachure_angle, hachure_gap, seed,
                disable_multi_stroke, disable_multi_stroke_fill, preserve_vertices, fill_shape_roughness_gain,
                stroke_line_dash: Option<[f64;2]>, fill: Option<Color>, stroke: Option<Color> }
                // plus rng: Option<RoughRng>; shared via &mut, cloned only where JS uses cloneOptionsAlterSeed
  ops.rs        enum Op { Move(P), Bezier(P,P,P), Line(P) }; struct OpSet { kind: Path|FillPath|FillSketch, ops: Vec<Op> }
  renderer.rs   line/linear_path/polygon/rectangle/curve/ellipse/svg_path/solid_fill_polygon (1:1 with renderer.js)
  hachure.rs    hachure_lines + fillers
  path.rs       path-data-parser (parse, absolutize, normalize) + points_on_path + points_on_bezier_curves + rdp
  generator.rs  Generator::{rectangle, ellipse, polygon, linear_path, curve, path, line, circle} -> Drawable { shape, sets, options }
```

Test it by porting a handful of roughjs outputs: run `node -e` on 4.6.4 with fixed seeds and compare the op arrays to `1e-9`.

---

## 3. Arrow binding (`packages/element/src/binding.ts`)

### 3.1 Constants (`binding.ts:117-143`)

```
BASE_BINDING_GAP     = 5
getBindingGap(t)     = 5 + t.strokeWidth / 2          // the arrow endpoint sits this far outside the target outline
BASE_ARROW_MIN_LENGTH = 10
FOCUS_POINT_SIZE     = 10/1.5
MIN_BINDABLE_SIZE    = 1
maxBindingDistance_simple(zoom) = clamp(15 / (z*1.5), 15, 30), where z = zoom<1 ? zoom : 1
FIXED_POINT_BOUND    = 10                              // fixedPoint ratios are clamped to [-10, 10]
```

Bindable types: rectangle, stickynote, diamond, ellipse, text, image, iframe, embeddable, frame, magicframe (`types.ts:293-303`).

### 3.2 Hit-testing a binding target (`collision.ts:307-470`)

`getBindingCandidates(point)` iterates elements **front to back**:

- Skip non-bindable elements.
- Frames are only bindable from outside (inside gives `-inf`).
- A point clipped by an enclosing frame is skipped.
- `distance`: inside gives `+distanceToOutline`; outside within `maxBindingDistance` gives `-distance`; otherwise `-inf`.
- Candidate if `distance > -maxDist` and not locked.
- Stop at the first *opaque* element containing the point (images, or shapes with a non-transparent background).

`getHoveredElementForBinding`:
- 0 candidates gives none; 1 gives that one.
- Otherwise sort by `|distance|`, take the closest, then prefer a smaller overlapping candidate that contains the point
  (`overlapPercent > 0.25 && relativeArea < 0.75`).

### 3.3 FixedPoint (`binding.ts:2196-2247, 2670-2787`)

```
fixedPointFor(arrowEndpoint P, target T):
   q = rotate(P, centre(T), -T.angle)
   if T.w < 1 || T.h < 1: return normalize([0.5, 0.5])
   floor = getBindingGap(T)
   return normalize([(q.x - T.x) / max(T.w, floor), (q.y - T.y) / max(T.h, floor)])
normalizeFixedPoint(fp): if !finite -> [0.5001, 0.5001]; clamp each to [-10,10];
                         any coord with |c - 0.5| < 1e-4 becomes 0.5001      // avoids heading flips
globalFixedPoint(fp, T) = rotate((T.x + T.w*fx, T.y + T.h*fy), centre(T), T.angle)
```

`bindBindingElement` (`binding.ts:1141-1185`):
- Elbow arrows always get `mode "orbit"` with a fixedPoint from `calculateFixedPointForElbowArrowBinding` (snaps to the outline first).
- Simple arrows take the strategy's mode and a fixedPoint from the **focus point**: the pointer position, or the origin for the start.
- `applyBinding` (`binding.ts:1119-1139`) sets `arrow.startBinding/endBinding` and appends `{id, type:"arrow"}` to `target.boundElements` if missing.
- `unbindBindingElement` (`binding.ts:1187-1217`) removes the back-ref unless the other end is bound to the same element.

Mode selection when drawing a new arrow (`bindingStrategyForNewSimpleArrowEndpointDragging`, `binding.ts:319-485`):
- The start point over a shape gives `inside`.
- The end over the same shape as the start gives inside/inside (or orbit for multi-point arrows).
- The end over another shape gives `orbit`, unless the global bind mode is `inside`/`skip` (Alt/Ctrl toggles) and the shapes are not nested.
- The start binding becomes `orbit` if the arrow left the start shape, unless the start was flagged `arrowStartIsInside`.

Meaning of the modes:
- **inside**: the endpoint sits exactly at the fixed point (it can be inside the shape).
- **orbit**: the endpoint goes to the outline (offset by the gap) along the line towards the other end.
- **skip**: never bind.

### 3.4 Outline intersection

`intersectElementWithLineSegment(el, seg, offset)` (`collision.ts:495-559`):
- AABB prefilter on the segment expanded by `offset`.
- Rectanguloids: rotate the segment by `−angle` about the centre and intersect with the 4 sides plus 4 corner cubics of the **offset** outline,
  then rotate the hits back (`collision.ts:647-700`).
- Diamond: same idea using `deconstructDiamondElement`.
- Ellipse: analytic intersection with `ellipse(centre, w/2+offset, h/2+offset)`.
- Linear: intersect with the rough shape's lines and curves.

`deconstructRectanguloidElement(el, offset)` (`utils.ts:245-344`):
- `r = getCornerRadius(min(w,h))`, or `0.01` if 0.
- The sides are inset by `r`.
- Each corner is the cubic `side_end → side_end + 2/3 (corner − side_end) → … → next_side_start`.
- With `offset > 0` each corner cubic is offset (`curveOffsetPoints`), re-fitted with Catmull-Rom (`curveCatmullRomCubicApproxPoints`),
  and the sides reconnect the curve ends.

For a port it is fine to approximate: offset the rounded rect / diamond / ellipse polygon by `gap` and intersect the segment with it.

### 3.5 Updating bound arrows when a shape moves (`updateBoundElements`, `binding.ts:1321-1429`)

For each `boundElements` entry of the changed element:
- Take the arrow; skip deleted ones and arrows that are themselves being moved in the same operation.
- For each end bound to the changed element (or to the other end's element when the arrow has 2 points), compute a new endpoint with `updateBoundPoint`.
- Then `LinearElementEditor.movePoints(arrow, {idx -> point})`, which renormalises x/y/points/size and re-routes elbow arrows via `updateElbowArrowPoints`.
- Re-position the arrow label.

`updateBoundPoint(arrow, end, binding, T)` (`binding.ts:1972-2128`):

```
return None if no binding, or (binding.elementId != T.id && points > 2), or last point == (0,0) (arrow just created)
focus = globalFixedPoint(binding.fixedPoint, T)
if mode == inside: return focus
other     = binding of the opposite end (element + its focus point)
otherPt   = arrow point index 1 (for start) / len-2 (for end), global
anchor    = points == 2 ? (otherFocus or otherPt) : otherPt
seg       = [focus, anchor]
outline   = intersect(T, seg, gap(T)) sorted by distance to anchor, first
otherOut  = intersect(otherEl, seg, gap(otherEl)) sorted by distance to focus, first
resolvedTarget = (no arrowheads at all) || (this end has an arrowhead) ? focus : (outline or focus)
1. if otherEl && outline && !dragging && area(otherEl) < 2*area(T) && outline hits otherEl (threshold gap): return resolvedTarget
2. tooShort = dist(otherOut or otherFocus or otherPt, outline or focus) <= 10
   if no otherEl: return tooShort ? focus : (outline or focus)
3. if tooShort: return resolvedTarget
4. return outline or focus
```

The arrow is drawn along the line between the two focus points and clipped to each target's outline plus gap. That is the core "orbit" behaviour.

Suggested Rust shape:

```rust
pub enum BindMode { Inside, Orbit, Skip }
pub struct Binding { pub element_id: Id, pub fixed_point: [f64; 2], pub mode: BindMode }
pub fn binding_gap(stroke_width: f64) -> f64 { 5.0 + stroke_width / 2.0 }
pub fn update_bound_point(arrow: &Arrow, end: End, b: &Binding, target: &Shape, scene: &Scene) -> Option<Vec2>;
```

---

## 4. Elbow arrows (`packages/element/src/elbowArrow.ts`)

Constants: `BASE_PADDING = 40` (`:111`), `DEDUP_TRESHOLD = 1` (`:110`), `MAX_POS = 1e6` (`:902`), corner radius 16 at render time.
Headings (`heading.ts:31-35`): `RIGHT=[1,0] DOWN=[0,1] LEFT=[-1,0] UP=[0,-1]`.

```
vectorToHeading(v):  x >  |y| -> RIGHT ; x <= -|y| -> LEFT ; y > |x| -> DOWN ; else UP      (heading.ts:37-49)
```

`headingForPointFromElement(el, aabb, p)` (`heading.ts:231-287`):
- Non-diamond: scale the aabb corners ×2 about its centre and test which of the 4 triangles `(corner, corner, centre)` contains `p`, in the order UP, RIGHT, DOWN, else LEFT.
- Diamond: use the vertices shrunk ×0.95 and cross-product sector tests (`heading.ts:69-226`).

### 4.1 Entry point `updateElbowArrowPoints(arrow, scene, updates, opts)` (`:907-1190`)

1. If the arrow has fewer than 2 points, return the points unchanged.
2. `updatedPoints`: if `updates.points` has 2 points, replace only the first and last of the existing points.
3. If a bound element is missing (or the scene is empty) and the points are already orthogonal (`validateElbowPoints`, tolerance 1), just normalise.
4. `data = getElbowArrowData(...)` (§4.2).
5. No updates at all: `handleSegmentRenormalization` (merge collinear and short segments, keep fixed segments).
6. `fixedSegments` empty: **full route**, then
   `normalize(getElbowArrowCornerPoints(removeElbowArrowShortSegments(routeElbowArrow(...))))`.
7. Otherwise use the fixed-segment handlers:
   - `handleSegmentRelease`: re-route only the sub-path between the neighbouring fixed segments with `routeElbowArrow`.
   - `handleSegmentMove`: translate the dragged segment and re-align its neighbours; no A*.
   - `handleEndpointDrag`: keep the interior points and recompute the first/last legs.

### 4.2 `getElbowArrowData` (`:1192-1435`)

```
origStart/End = arrow.(x,y) + nextPoints[0 / last]
hoveredStart/End = dragging ? hit-test (§3.2) : bound elements by id
startP = dragging&&bound ? bindPointToSnapToElementOutline(...) : bound ? globalFixedPoint(fixedPoint || [0,0]) : origStart
startHeading = getHeadingForElbowArrowSnap(startP, endP, hoveredStart, aabbForElement(el, pad all sides = distanceToElement(el, startP)), origStart):
     no element                            -> vectorToHeading(endP - startP)
     distance(origStart, el) > maxBindDist -> vectorToHeading(startP - centre(el))
     else                                  -> headingForPointFromElement(el, aabb, startP)
pointBounds(p)   = [p.x-2, p.y-2, p.x+2, p.y+2]
elementBounds    = aabbForElement(el, offsetFromHeading(heading, arrowhead ? gap*6 : gap*2, 1))   // gap = 5 + sw/2
offsetFromHeading(h, head, side) -> [top,right,down,left] with `head` on the side h points to, `side` elsewhere
aabbForElement(el, [t,r,d,l]) = rotated-box AABB expanded: [minX-l, minY-t, maxX+r, maxY+d]
boundsOverlap = inside(startP, endEl ? aabb(endEl, offsetFromHeading(endH, 40, 40)) : endPointBounds)
             || inside(endP, startEl ? aabb(startEl, offsetFromHeading(startH, 40, 40)) : startPointBounds)
             (inside = STRICT <, bounds.ts:1229)
common = commonAABB(overlap ? [startPB, endPB] : [startEB, endEB])
noneBound = !startEl && !endEl
startDiff = overlap ? offsetFromHeading(startH, noneBound?0:40, 0)
                    : offsetFromHeading(startH, noneBound?0: 40 - (startArrowhead ? 30 : 10), 40)
endDiff   = same with end
dynamicAABBs = generateDynamicAABBs(overlap?startPB:startEB, overlap?endPB:endEB, common, startDiff, endDiff,
                                    disableSideHack = overlap, aabb(startEl), aabb(endEl))
startDongle = getDonglePosition(dynamicAABBs[0], startH, startP):
     UP -> (p.x, b.minY) ; RIGHT -> (b.maxX, p.y) ; DOWN -> (p.x, b.maxY) ; LEFT -> (b.minX, p.y)
```

### 4.3 `generateDynamicAABBs(a, b, common, sDiff=[su,sr,sd,sl], eDiff=[eu,er,ed,el], disableSideHack, aEl=a, bEl=b)` (`:1666-1849`)

Two boxes that grow toward each other until they touch at the midline between the elements:

```
first = [
  a.x0 > b.x2 ? ((a.y1 > b.y3 || a.y3 < b.y1) ? min((aEl.x0 + bEl.x2)/2, a.x0 - sl) : (aEl.x0 + bEl.x2)/2)
              : a.x0 > b.x0 ? a.x0 - sl : common.x0 - sl,
  a.y1 > b.y3 ? ((a.x0 > b.x2 || a.x2 < b.x0) ? min((aEl.y1 + bEl.y3)/2, a.y1 - su) : (aEl.y1 + bEl.y3)/2)
              : a.y1 > b.y1 ? a.y1 - su : common.y1 - su,
  a.x2 < b.x0 ? ((a.y1 > b.y3 || a.y3 < b.y1) ? max((aEl.x2 + bEl.x0)/2, a.x2 + sr) : (aEl.x2 + bEl.x0)/2)
              : a.x2 < b.x2 ? a.x2 + sr : common.x2 + sr,
  a.y3 < b.y1 ? ((a.x0 > b.x2 || a.x2 < b.x0) ? max((aEl.y3 + bEl.y1)/2, a.y3 + sd) : (aEl.y3 + bEl.y1)/2)
              : a.y3 < b.y3 ? a.y3 + sd : common.y3 + sd ]
second = the same with (a,b,aEl,bEl,s*) swapped to (b,a,bEl,aEl,e*)
```

(Indices: `x0=[0] y1=[1] x2=[2] y3=[3]`, i.e. `[minX, minY, maxX, maxY]`.)

Side hack (when `!disableSideHack` and the two boxes overlap on both axes, i.e. `w1+w2 > common.w + 1e-11 && h1+h2 > common.h + 1e-11`):
for the diagonal quadrant configurations (b is bottom-left, top-left, top-right or bottom-right of a), split along the shared midline
`cX`/`cY` chosen by the sign of `cross(a-corner − endCentre, other a-corner − endCentre)` (`:607-701`).
Port this verbatim; it is pure arithmetic.

### 4.4 Grid and A* (`:1437-1660, 1851-2080`)

`calculateGrid(aabbs, start, startH, end, endH, common)`:

```
xs = set(), ys = set()
startH horizontal ? ys.add(start.y) : xs.add(start.x)     // same for end
for each aabb: xs.add(x0, x2); ys.add(y1, y3)
xs.add(common.x0, common.x2); ys.add(common.y1, common.y3)
sort ascending; nodes at every (x, y); addr = [col, row]; data row-major (row * cols + col)
```

`routeElbowArrow`:
- `startDongle = pointToGridNode(startDongle)` (exact float equality scan).
- The end node is closed if the end is bound; the start node is closed if the start is bound.
- `dongleOverlap = inside(startDongle, aabbs[1]) || inside(endDongle, aabbs[0])`. If true, the obstacles become `[]`.
- `path = astar(startDongle, endDongle, grid, startH, endH, obstacles)`, then `points = [startP, ...path.pos, endP]`.

`astar` (`:1535-1644`):

```
bend = manhattan(start.pos, end.pos)
open = BinaryHeap by f (min); push start
while let Some(cur) = open.pop():
   if cur.closed: continue
   if cur == end: return pathTo(start, cur)        // walk parents, prepend start
   cur.closed = true
   for (i, nb) in [up(row-1), right(col+1), down(row+1), left(col-1)]:
       if nb none or closed: continue
       mid = (nb.pos + cur.pos) / 2 ; if any obstacle strictly contains mid: continue
       nh = [UP,RIGHT,DOWN,LEFT][i]
       prev = cur.parent ? vectorToHeading(cur.pos - parent.pos) : startH
       if nh == flip(prev) || (nb == start && nh == startH) || (nb == end && nh == endH): continue
       g = cur.g + manhattan(nb, cur) + (prev != nh ? bend^3 : 0)
       if !nb.visited || g < nb.g:
           nb.visited = true; nb.parent = cur
           nb.h = manhattan(end, nb) + estimateSegmentCount(nb, end, nh, endH) * bend^2
           nb.g = g; nb.f = g + nb.h
           if newly visited: push else rescore (decrease-key)
return None
```

The heap is the classic Eloquent-JS heap (`common/src/binary-heap.ts`) with a strict `<` in bubble-up and sink-down. Port it 1:1 to keep identical tie-breaks.
`std::collections::BinaryHeap` with lazy deletion can give different but equally valid routes.

`estimateSegmentCount(nodeStart, nodeEnd, sH, eH)` (`:1924-2040`), where s = start node pos and e = end node pos:

| eH \ sH | RIGHT | UP | DOWN | LEFT |
|---|---|---|---|---|
| RIGHT | s.x>=e.x ? 4 : s.y==e.y ? 0 : 2 | (s.y>e.y && s.x<e.x) ? 1 : 3 | (s.y<e.y && s.x<e.x) ? 1 : 3 | s.y==e.y ? 4 : 2 |
| LEFT | s.y==e.y ? 4 : 2 | (s.y>e.y && s.x>e.x) ? 1 : 3 | (s.y<e.y && s.x>e.x) ? 1 : 3 | s.x<=e.x ? 4 : s.y==e.y ? 0 : 2 |
| UP | (s.y>e.y && s.x<e.x) ? 1 : 3 | s.y>=e.y ? 4 : s.x==e.x ? 0 : 2 | s.x==e.x ? 4 : 2 | (s.y>e.y && s.x>e.x) ? 1 : 3 |
| DOWN | (s.y<e.y && s.x<e.x) ? 1 : 3 | s.x==e.x ? 4 : 2 | s.y<=e.y ? 4 : s.x==e.x ? 0 : 2 | (s.y<e.y && s.x>e.x) ? 1 : 3 |

### 4.5 Post-processing (`:2102-2200`)

- `removeElbowArrowShortSegments`: if there are at least 4 points, drop interior points with `dist(prev_original, p) <= 1`.
- `getElbowArrowCornerPoints`: keep the first and last points; drop an interior point if the segment into it and the segment out of it
  have the same "horizontal-ness" (`|dy| < |dx|`), i.e. drop collinear points.
- `normalizeArrowElementUpdate`: `x,y = first point`, points become relative, clamped to ±1e6, `width/height` from the points,
  `fixedSegments = null` if empty.
- `validateElbowPoints(points, tol=1)`: every consecutive pair shares x or y within `tol`.

Rendering: §2.2 `generateElbowArrowShape` (radius 16, path, `preserveVertices`).

Suggested Rust: `struct Grid { xs: Vec<f64>, ys: Vec<f64>, nodes: Vec<Node> }`,
`struct Node { f, g, h, closed, visited, parent: Option<usize>, pos: Vec2, addr: (usize, usize) }`,
and an index-based heap with `rescore(idx)` (keep a `heap_pos: Vec<usize>` side table, or linear-search like JS).
The grid is small (tens of lines), so O(n) rescoring is fine.

---

## 5. Laser pointer (`packages/laser-pointer/src/*`, `packages/excalidraw/laserTrails.ts`, `animatedTrail.ts`)

`LaserPointer` (`state.ts:26-41`):
- defaults `size 2, streamline 0.45, simplify 0.1, simplifyPhase "output", keepHead false, sizeMapping = () => 1`.
- constants `cornerDetectionMaxAngle = 75°`, `cornerDetectionVariance(speed) = speed > 35 ? 0.5 : 1`, `maxTailLength = 50`.

Points are `[x, y, r]`. For the laser trail, `r` is the **timestamp** (`performance.now()`) at which the point was added (`animatedTrail.ts:114-127`).

`addPoint(p)` (`state.ts:62-86`):
- Ignore exact duplicates of the last original point.
- The first point goes into `stablePoints`.
- Otherwise `p = lerp(lastPoint, p, 1 - streamline)`, then push to `tailPoints`.
- If `runLength(tail) > 50`, move the tail into stable.
- `runLength` sums the segment lengths **and adds the last segment again** (`math.ts:65-79`). Port that quirk.

Excalidraw's laser options (`laserTrails.ts:24-42`): `simplify: 0, streamline: 0.4`, and

```
sizeMapping(c) = min(easeOut(l), easeOut(t))
   t = max(0, 1 - (now - c.pressure) / DECAY_TIME)                    // DECAY_TIME = 1000 ms; pressure == timestamp
   l = (DECAY_LENGTH - min(DECAY_LENGTH, totalLength - currentIndex)) / DECAY_LENGTH   // DECAY_LENGTH = 50 (points, not px)
   easeOut(k) = 1 - (1 - k)^4                                          // common/src/utils.ts:232
```

- Each point shrinks to zero 1 s after it was drawn. Only the newest 50 points are visible, tapering from full size at the head to 0 at 50 points back.
- Colour `DEFAULT_LASER_COLOR = "red"` (`constants.ts:38`). Collaborators use their client colour.
- The trail is drawn with `size = options.size / zoom` (constant on-screen width, base size 2) every animation frame.
- Finished trails stay in `pastTrails` until their outline is empty. The animation stops when no paths remain.

`getStrokeOutline(sizeOverride)` (`state.ts:119-377`), with `len = points.len()`:

- `len == 1`: a circle polygon of radius `size` (θ step π/16).
- `len == 2`: two half-circles (a capsule).
- Otherwise, for `i in 1..len-1` with `p=pts[i-1], c=pts[i], n=pts[i+1]`:

  ```
  d = |p-c| ; runningLength += d ; speed = prevSpeed + (d - prevSpeed)*0.2
  cSize = size * sizeMapping({pressure: c.r, runningLength, currentIndex: i, totalLength: len})
  if cSize == 0: visibleStartIndex = i + 1; continue
  dirPC = norm(p-c), dirNC = norm(n-c); p1dirPC = rot(dirPC, +π/2), p2dirPC = rot(dirPC, -π/2), p1dirNC, p2dirNC likewise
  p1PC = c + p1dirPC*cSize, p2PC, p1NC, p2NC likewise
  ftdir = p1dirPC + p2dirNC ; btdir = p2dirPC + p1dirNC
  paPC = c + (|ftdir|==0 ? dirPC : norm(ftdir)) * cSize ; paNC = c + (|btdir|==0 ? dirNC : norm(btdir)) * cSize
  cAngle = normAngle(angle(c, p, n))           // atan2(n-c) - atan2(p-c), wrapped to (-π, π]
  D = 75/180*π * (speed > 35 ? 0.5 : 1)
  if |cAngle| < D:                              // sharp corner -> round join with 5 arc samples (step tAngle/4)
      tAngle = |normAngle(π - cAngle)| ; if tAngle == 0 continue
      if cAngle < 0: back += [p2PC, paNC]; fwd += arc(c, p1dirPC*cSize, 0..tAngle); back += arc(..., tAngle..0); back += [paNC, p1NC]
      else:          fwd += [p1PC, paPC]; back += arc(c, -p1dirPC*cSize, rotate by -θ, 0..tAngle); fwd += arc(..., tAngle..0); fwd += [paPC, p2NC]
  else: fwd.push(paPC); back.push(paNC)
  prevSpeed = speed
  ```

- If `visibleStartIndex >= len - 2`: return a head circle if `keepHead`, otherwise empty.
- Start cap: a half-circle at `first` (θ 0..π, step π/16, radius = `size(first)`, only if it is > 0.1).
- End cap: θ 0..3π (step π/16) around `ultimate` with radius = `keepHead ? size : size(penultimate)`.
- Outline = `startCap ++ fwd ++ reverse(endCap) ++ reverse(back) ++ [startCap[0]]`, then RDP if `simplifyPhase == "output"` (not used: simplify is 0).
- Rendered as SVG via `getSvgPathFromStroke(points, closed=true)` (`common/src/utils.ts:1012-1042`):
  `M p0 Q p1 avg(p1,p2) T avg(p2,p3) avg(p3,p4) … Z`, returning empty for fewer than 4 points.

Rust shape: `struct LaserTrail { stable: Vec<[f64;3]>, tail: Vec<[f64;3]>, original: Vec<[f64;3]>, opts }`.
Call `outline(now, zoom) -> Vec<Vec2>` each frame and tessellate the polygon (non-convex: use lyon fill or an egui mesh via earcut).

---

## 6. Library

### 6.1 `.excalidrawlib` (`data/json.ts:128-145`, `types.ts:650-665`)

```json
{ "type": "excalidrawlib", "version": 2, "source": "https://excalidraw.com",
  "libraryItems": [ { "id": "…", "status": "published" | "unpublished", "created": 1700000000000,
                      "name": "optional", "elements": [ ExcalidrawElement, … ] } ] }
```

- Version 1 used `"library": [[el, el], [el]]`, an array of element arrays. Accept `version` 1 or 2.
- `restoreLibraryItems` (`restore.ts:1381-1414`) wraps v1 arrays into items (`id = randomId()`, `created = now`, `status = default`).
- Each item's elements pass through `restoreElements` and are filtered to non-deleted. Items with no elements are dropped.
- Library items cannot contain `iframe`, `embeddable` or `image` (`LIBRARY_DISABLED_TYPES`, `constants.ts:545`).
- MIME: `application/vnd.excalidrawlib+json`.

### 6.2 Inserting items

- Several items at once go through `distributeLibraryItemsOnSquareGrid` (`data/library.ts:405-490`):
  `ITEMS_PER_ROW = ceil(sqrt(n))`, `PADDING = 50`. Each item is placed in a cell sized by its row's max height and column's max width,
  centred in the cell, with its own bbox moved to the origin.
- `addElementsFromPasteOrLibrary(position: "center")` (`components/App.tsx:4587-4690`):
  1. `restoreElements(…, {deleteInvisibleElements: true})`.
  2. `duplicateAtSceneCoords(elements, viewportCentreInScene)` (`components/App.duplicate.ts:270-342`):
     `dx = x − bboxW/2`, `dy = y − bboxH/2`, snap `(dx,dy)` to the grid if enabled, then `el.x += gridX − minX` and `el.y += gridY − minY`.
     The bbox is therefore centred on the target.
  3. `duplicateElements` gives **new ids**, new groupIds (consistent remap), remapped bindings, `boundElements`, `containerId` and `frameId`,
     and **randomizes seeds** unless `retainSeed`.
  4. Append to the scene and assign fractional indices after the current top (`syncMovedIndices`).
  5. If the drop point is inside a frame, add the eligible elements to that frame.
  6. Re-layout bound text and select the inserted elements.

Rust: `fn insert_library_item(scene, item, at: Vec2)` should do the same id-remap pass as paste. Share one `duplicate_with_remap` helper for paste, library and alt-drag.

---

## 7. Shapes, diamond, text containers, sticky notes

### 7.1 Tools and types

Tools (`TOOL_TYPE`, `constants.ts:552-572`): selection, lasso, rectangle, diamond, ellipse, arrow, line, freedraw, text, image, eraser, hand, frame,
magicframe, stickynote, embeddable, laser, autoshape, bucketfill.

Element types: see §1.1. Arrow sub-types are `sharp` (`roundness: null`), `round` (`roundness: {type: 2}`) and `elbow` (`elbowed: true`).
Default roundness for new elements (`getDefaultRoundnessTypeForElement`, `typeChecks.ts:357-373`):
- rectangle/embeddable/iframe/image: ADAPTIVE (3).
- line/arrow/diamond: PROPORTIONAL (2).

### 7.2 Diamond geometry (`bounds.ts:522-535`)

```
topX = floor(w/2) + 1 ; topY = 0
rightX = w ; rightY = floor(h/2) + 1
bottomX = topX ; bottomY = h
leftX = 0 ; leftY = rightY
```

The `+1` exists only to keep roughjs from receiving zero-length edges. Keep it for pixel parity.
Rounded diamond: §2.2. Hit/bind geometry: `getDiamondBaseCorners` (`utils.ts:377-455`) uses corner radii
`vr = roundness ? getCornerRadius(|topX-leftX|) : (topX-leftX)*0.01` and `hr` likewise.

### 7.3 Text and bound text (`textElement.ts`)

Constants:
- `BOUND_TEXT_PADDING = 5` (`constants.ts:424`), `ARROW_LABEL_WIDTH_FRACTION = 0.7`, `ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO = 11`.
- Valid containers: rectangle, stickynote, ellipse, diamond, arrow (`textElement.ts:508-519`).

`getContainerCoords` (`:396-417`): `pad = 5` (16 for stickynote).
- ellipse: `off += (w/2)*(1 - √2/2)`, same for y.
- diamond: `off += w/4`, `h/4`.
- The result is `(x + offX, y + offY)`.

`getBoundTextMaxWidth` (`:540-569`):
- ellipse: `round(w/2*√2) − 10`.
- diamond: `round(w/2) − 10`.
- arrow: `max(0.7*w, fontSize*11)`.
- else: `w − 10` (stickynote `w − 32`).

`getBoundTextMaxHeight` (`:571-600`):
- ellipse: `round(h/2*√2) − 10`.
- diamond: `round(h/2) − 10`.
- stickynote: `max(0, h − 52)`.
- else: `h − 10`.

`computeBoundTextPosition` (`:249-324`), with `cc = containerCoords`, `mw/mh` = max sizes:
- `y` = top: `cc.y`; bottom: `cc.y + mh − th`; middle: `cc.y + mh/2 − th/2`.
- `x` = left: `cc.x`; right: `cc.x + mw − tw`; centre: `cc.x + mw/2 − tw/2`.
- If the container is rotated, rotate the text centre about the content centre `(cc + (mw/2, mh/2))`. The text `angle` = container angle (0 for arrows).

Growth (`redrawTextBoundingBox`, `:51-153`):
- Wrap `originalText` to `maxWidth`.
- If the text height is greater than `maxHeight`, the container height becomes `computeContainerDimensionForBoundText(h)`; the same applies to width.

`computeContainerDimensionForBoundText(d)` (`:521-538`), with `d = ceil(d)` and `pad = 10`:
- ellipse: `round((d + 10)/√2 * 2)`.
- diamond: `2*(d + 10)`.
- arrow: `d + 80`.
- else: `d + 10`.

Text rendering baseline (`font-metadata.ts:155-170`):

```
lineHeightPx = fontSize * lineHeight
verticalOffset = fontSize/unitsPerEm*ascender + (lineHeightPx - fontSize/unitsPerEm*ascender + fontSize/unitsPerEm*descender)/2
line i baseline at y + i*lineHeightPx + verticalOffset
```

Font metrics (unitsPerEm / ascender / descender):
- Excalifont & Virgil: 1000 / 886 / −374
- Nunito: 1000 / 1011 / −353
- Helvetica: 2048 / 1577 / −471
- Cascadia: 2048 / 1900 / −480
- Comic Shanns: 1000 / 750 / −250
- Lilita One: 1000 / 923 / −220
- Liberation Sans: 2048 / 1854 / −434

Arrow labels: positioned at `labelPosition` (normalised arc length, default 0.5, `textElement.ts:617`) along the arrow path. The arrow stroke is masked behind the label.

### 7.4 Sticky-note-like behaviour

At this commit Excalidraw has a real `stickynote` element (`stickyNote.ts`, constants `constants.ts:226-268`). It is useful as a spec.

Container:
- Always `fillStyle: "solid"`, with a non-transparent background (default `#ffdf6b`) and stroke (default `#1e1e1e`).
- Default size 250×250, min 75. `baseHeight` is the user-set height; the note **grows** to fit the label and never shrinks below `baseHeight`.

Label: the bound text, `verticalAlign` middle.
- Padding 16. A 20px creation-date footer is reserved (body inset Y = 16*2 + 20 = 52).
- Font auto-fits: it shrinks from `baseFontSize` (fallback 28) in steps of 2, down to min 16 (`STICKY_NOTE_*`).

Look:
- Painted directly, not with roughjs.
- Corners are jittered by `min([0, 1.5, 8][round(roughness)], min(w,h)*0.012)` using mulberry32 `seededRandom(seed)` (`common/src/random.ts`).
- Corner radius `min(min(w,h)*0.04, 16)` when `roundness` is set.
- Shadow offset 3 at opacity 0.16, plus an edge shadow (width 0.5, opacity 0.08).
- The note's stroke colour = the label colour ("ink").

For Sketch, model it as `Shape::StickyNote { base_height }` plus a bound `Text`. Reuse the bound-text layout from §7.3 with padding 16. Implement grow-to-fit by setting `h = max(baseHeight, textH + 52)`.

For older files without stickynote, the equivalent is a rectangle (solid fill, yellow background, `roundness {type: 3}`) with bound text in `verticalAlign: middle` / `textAlign: center`.

---

## 8. Other details useful to a port

### 8.1 Fractional indexing for z-order (`packages/fractional-indexing/src/index.ts`, `element/src/fractionalIndex.ts`)

- Keys are base-62 strings (`0-9A-Za-z`, ASCII order).
- The integer part's length is set by its head char: `a..z` gives `head-'a'+2` chars (`a0`..`az`, `b00`..), `A..Z` gives `'Z'-head+2` (negative side).
- The fractional part must not end in `'0'`.
- Comparison is plain byte-wise string compare (`a < b`).
- `generateKeyBetween(None, None) = "a0"`. Then:
  - `between(a, None)` increments the integer part (`a0` → `a1` … → `az` → `b00`).
  - `between(None, b)` decrements it.
  - Otherwise take the midpoint of the fractional parts: e.g. `midpoint("", "V") = "G"` (digit index `Math.round(0.5*(0+31)) = 16`).
- `generateNKeysBetween(a, b, n)` recursively bisects (`:270-322`).
- Scene invariant: `elements[i].index < elements[i+1].index`. Ties are broken by `id` when sorting (`orderByFractionalIndex`, `fractionalIndex.ts:150-169`).
- After moves, `syncMovedIndices` regenerates keys only for the moved runs, between their neighbours. `syncInvalidIndices` fixes any invalid ones; restore calls it.
- Rust: port the ~320-line file directly; it is self-contained (CC0).

### 8.2 Versioning and reconciliation

- Each mutation does `version += 1`, `versionNonce = randomInteger()`, `updated = now` (`mutateElement.ts:142-144`).
- Remote vs local: keep local if `local.version > remote.version`, or the versions are equal and `local.versionNonce <= remote.versionNonce` (`reconcile.ts:23-44`).
- On import, `bumpElementVersions` raises imported versions above local copies with the same id (`restore.ts:1150-1173`).

### 8.3 Groups

- `groupIds` is ordered **innermost → outermost**. The outermost group is `groupIds.last()`.
- Clicking an element selects the outermost group not inside the group being edited:
  slice `groupIds` before `editingGroupId`, then take `.last()` (`groups.ts:94-121`).
- Double-click enters `editingGroupId = groupIds[0]` (the innermost group, `groups.ts:123-135`).
- Grouping inserts the new id *before* `editingGroupId` in each element's list, or pushes it to the end if not editing (`addToGroup`, `groups.ts:162-176`).
  A new group therefore wraps the existing ones (it becomes the outermost).
- A group with fewer than 2 members is not selectable as a group (`groups.ts:37-49`).

### 8.4 Restore defaults and migrations (`packages/excalidraw/data/restore.ts`)

`restoreElementWithProperties` (`:430-515`) normalises every element:

```
version || 1 ; versionNonce ?? 0 ; index ?? null ; isDeleted ?? false ; id || randomId()
fillStyle || "solid" ; strokeWidth || 2 ; strokeStyle ?? "solid" ; roughness ?? 1 ; opacity ?? 100 (null -> 100)
angle || 0 ; x ?? 0 ; y ?? 0 ; strokeColor || "#1e1e1e" ; backgroundColor || "transparent"
width || 0 ; height || 0 ; seed ?? 1 ; groupIds ?? [] ; frameId ?? null ; updated ?? now ; created ?? null
link: normalizeLink or null ; locked ?? false
roundness: element.roundness
        || (legacy strokeSharpness == "round" ? { type: isUsingAdaptiveRadius(type) ? 1 (LEGACY) : 2 } : null)
boundElements: legacy boundElementIds -> [{type:"arrow", id}] else boundElements ?? []
negative width/height -> abs + shift x/y (getNormalizedDimensions, sizeHelpers.ts:256)
unknown keys are preserved (spread) ; strokeSharpness / boundElementIds deleted
```

Per-type rules (`restoreElement`, `:517-759`):

- **text**: drop `rawText`. Legacy `font: "20px Virgil"` is parsed into fontSize + family. `fontSize` defaults to 20.
  `lineHeight` falls back to a value detected from height when height is set, otherwise to the family default.
  `textAlign || "left"`, `verticalAlign || "top"`, `originalText || text`, `autoResize ?? true`, `labelPosition` clamped to 0..1.
- **freedraw**: drop invalid points. `pressures[i]` defaults to 0.5 if not finite. `strokeOptions` defaults to `{variability:"variable", streamline:0.5}`.
- **image**: `status || "pending"`, `scale || [1,1]`, `crop ?? null`.
- **line** (and legacy type **`draw`**, which becomes `line`):
  - Fewer than 2 valid points: points become `[[0,0],[w,h]]`.
  - If `points[0] != [0,0]`, shift so it is.
  - Bindings are nulled. Arrowheads are normalised.
  - `polygon` is kept only if the points form a valid polygon.
  - Size comes from the points.
- **arrow**:
  - `endArrowhead` undefined becomes `"arrow"` (old arrows).
  - Bindings go through `repairBinding` (`:298-428`):
    - Elbow: keep it, `mode || "orbit"`, normalise the fixedPoint.
    - v2 (has `mode`): keep `{elementId, mode, fixedPoint}`.
    - **v1 legacy** (focus/gap): if the endpoint is inside the target, `mode = inside` and the focus is the endpoint.
      Otherwise `orbit`, with the endpoint projected onto the target's diagonal. The fixedPoint is computed from that point (§3.3).
  - Elbow arrows keep `fixedSegments` only if there are at least 4 points.
  - Points are normalised.
- **Oversized** lines/arrows (w or h > 75 000) are replaced by a deleted 100×100 stub.
- **stickynote**: `baseHeight ?? maxHeight ?? height`.
- **frame/magicframe**: `name ?? null`.
- `selection` elements are dropped; unknown types are dropped.

`restoreElements(..., {repairBindings: true, refreshDimensions})` (`:946-1148`), in order:
1. Drop dangling `frameId`s.
2. Fix text `containerId` vs container `boundElements` in both directions. Bound text takes the container's angle (0 for arrows).
3. Null arrow bindings whose target is missing.
4. Sticky note ink/font invariants.
5. Move bound text right after its container.
6. Re-route invalid elbow arrows.

`syncInvalidIndices` assigns or repairs `index` for every element.

---

## 9. Fixtures written

These are hand-authored for Sketch (not copied from the repo) and checked to be valid JSON with internally consistent references.

`tests/fixtures/excalidraw/basic.excalidraw`, in array order (= z-order):

| id | type | notes |
|---|---|---|
| `rect-1` | rectangle | 100,100 200×100, hachure, ADAPTIVE roundness; boundElements = text + arrow |
| `text-bound-1` | text | `containerId: rect-1`, centre/middle, position computed per §7.3 (155, 137.5) |
| `ellipse-1` | ellipse | 450,100 150×100, cross-hatch, `frameId: frame-1`, bound arrow |
| `frame-1` | frame | 420,40 260×220, name "Frame A"; after its child per §1.4 |
| `arrow-1` | arrow | (306,150) → (444,150): rect right edge + gap 6 to ellipse left edge − gap 6 |
| `diamond-1` | diamond | dashed, roughness 2, solid fill, `groupIds: ["group-1"]` |
| `text-1` | text | standalone, `groupIds: ["group-1"]` (forms the 2-element group with the diamond) |
| `line-1` | line | 3 points, roughness 0, opacity 80 |
| `freedraw-1` | freedraw | 6 points + pressures, `simulatePressure: false` |
| `image-1` | image | 64×64 using a 1×1 PNG in `files`, `fileId` = SHA-1 of the bytes |

On `arrow-1`, both bindings carry the v2 fields (`fixedPoint`, `mode: "orbit"`) **and** the legacy `focus`/`gap`.
Current builds use the v2 fields; older builds read focus/gap.

`tests/fixtures/excalidraw/library.excalidrawlib`: version 2 with 2 items.
- "Button": a grouped rounded rectangle with bound text in Nunito.
- "Decision": a diamond plus an arrow with a triangle head.

Importer tests should assert, at minimum:
- element count and types;
- `text-bound-1.containerId == rect-1`, and the back-reference exists;
- `ellipse-1.frameId == frame-1`;
- the arrow bindings resolve, and with `fixedPoint ≈ 0.5` the endpoints lie on the gap-offset outlines;
- the group has 2 members;
- the image `fileId` resolves to a decodable PNG;
- the `index` keys are strictly increasing.
