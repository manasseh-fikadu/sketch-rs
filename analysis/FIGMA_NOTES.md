# Figma + Paper behavior notes (clean-room spec for Sketch)

Status: behavior spec only. Sources are public documentation (help.figma.com, developers.figma.com plugin/REST API docs, paper.design docs/compare pages) and general observed behavior of the public products. **No code, shaders, icons, fonts or assets from Figma or Paper are copied or referenced.**

Conventions:
* `Cmd` = Cmd on macOS, Ctrl on Windows/Linux. `Alt` = Option.
* "API:" names the Figma Plugin API property that documents the data model. Use these as field-name inspiration only. Sketch keeps its own names.
* **(verify)** marks rules recalled from product use that are not confirmed in a doc page fetched for this note. Check them against the live app before treating them as hard requirements.
* "Sketch note" lines are recommendations for this codebase (`src/model`, `src/editor`, `src/render`). They are not instructions to edit `src/` now.

Primary references:
* Plugin API node properties: https://developers.figma.com/docs/plugins/api/nodes/ (per-property pages under `/api/properties/`)
* REST file format: https://developers.figma.com/docs/rest-api/file-node-types/
* Help center: https://help.figma.com/hc/en-us
* Paper docs: https://paper.design/docs, https://paper.design/docs/svg, https://paper.design/docs/paste, https://paper.design/compare/figma

---

## 1. Frames vs groups

Sources: https://help.figma.com/hc/en-us/articles/360041539473 (Frames), https://help.figma.com/hc/en-us/articles/360039832054 (Groups), https://help.figma.com/hc/en-us/articles/360040449873 (Select layers), API `FrameNode.clipsContent`, `GroupNode`.

### Data model
| | Frame | Group |
|---|---|---|
| Own geometry | Has its own x/y/w/h, fills, strokes, effects, corner radius | None. Bounds are derived. |
| Bounds | Explicit and independent of children | **Union of children's (rotated) bounding boxes.** Recomputed after every child edit. |
| Clipping | `clipsContent` (UI "Clip content"). Default **on** for frames drawn with the Frame tool (verify for nested frames). | Never clips. |
| Fills | Yes, default white `#FFFFFF` 100% for a top-level frame drawn with F | No fills or strokes. Opacity, blend mode and effects apply to the group. |
| Constraints on children | Yes (§4) | Children behave as if constraints were Scale when the group is resized |
| Auto layout | Optional (§3) | No |
| Empty allowed | Yes | **No.** A group with 0 children is deleted automatically, including when its last child is deleted or moved out. |
| Rotation | Own rotation | Rotating a group rotates children about the group center. Group bounds become the AABB of the rotated children (verify how Figma stores this). |

### Creating
* **Group selection** `Cmd+G`: creates a group as a sibling at the z-index of the **top-most** selected layer, under the common parent. If the selected layers have different parents, all are moved into the parent of the top-most one (verify). The name defaults to "Group N".
* **Frame selection** `Cmd+Alt+G`: wraps the selection in a frame whose bounds are the selection's union bounds (no padding). The default fill is empty (verify). The name is "Frame N".
* **Ungroup** `Cmd+Shift+G`: replaces the group with its children, keeping their order and absolute positions. On a frame it also "removes the frame": children are reparented to the frame's parent, and the frame's own fill is lost.
* **Frame auto-adopt (draw):** drawing a new frame with the Frame tool (F/A) over existing layers adopts every *sibling-level* layer (same parent as the new frame) whose bounds are **fully contained** in the new frame's rect. Their absolute positions are preserved. Layers that are only partially covered are not adopted.
* **Frame adopt (drag):** dragging a layer and releasing it over a frame reparents it into the deepest frame under the **cursor** (not the layer's bounds). Groups never adopt by drop. Dragging a child out of a frame's bounds reparents it to the frame's parent (usually the canvas). Holding **Space** while dragging, or Cmd in some versions (verify which), prevents reparenting. The z-order on adopt is top of the new parent.
* Resizing a frame does not adopt or eject children. **Hold Cmd while resizing a frame to ignore its children's constraints** (children keep absolute positions).

### Selection model (click, deep select, keyboard)
* "Top-level frames" (direct children of the page) behave like artboards. **Clicking inside a top-level frame selects the direct child of that frame under the cursor**, not the frame itself. The frame is selected by clicking its **name label** above its top-left corner, by marquee-ing it from outside, or by clicking an empty area of a frame that has no children under the cursor.
* For anything nested deeper (a group, nested frame, component instance or boolean), a single click selects the **outermost selectable ancestor below the top-level frame**: the group or nested frame, not the leaf.
* **Selection context:** once a layer at depth N is selected, clicking another layer *inside the same parent* selects that sibling directly at depth N. Clicking outside the parent returns to the default rule.
* **Double-click** a group or frame selects the child under the cursor, one level deeper. On a vector it enters path edit mode. On text it enters text edit mode with the caret at the click position.
* **Cmd+click** (Ctrl on Windows) is deep select: it selects the deepest non-group layer under the cursor, ignoring containers. **Cmd+drag** marquee selects deep layers that intersect the marquee.
* **Enter**: select all direct children of the selection (on text: edit text; on vector: edit path).
* **Shift+Enter** or `\`: select the parent of each selected layer.
* **Tab / Shift+Tab**: next or previous sibling.
* **Shift+click**: toggle in the selection. **Esc**: deselect (verify whether one Esc goes to the parent first; Paper uses Esc to exit edit modes).
* **Marquee from empty canvas** selects layers at the **top level** whose bounds intersect the rect (partial touch is enough). A marquee that starts inside a top-level frame selects that frame's children that intersect it. Locked and hidden layers are never hit or marquee-selected.
* **Cmd+A**: select all siblings of the current selection (or all top-level layers on the page if nothing is selected).
* Right-click → "Select layer" lists every layer under the cursor in layer-panel order. Hidden layers are excluded, and locked ones show a padlock and are selectable there.

**Sketch note:** keep `Group` as a node kind with *derived* bounds (cached, invalidated on child change) and auto-delete when empty. Implement `hit_test(depth_policy)` with three policies (default/context, deep, child-of-selection) and a `selection_context: Option<NodeId>` in `Editor`. Frame adopt on draw = contained-sibling scan. Adopt on drag = deepest frame under the pointer, skipping the dragged nodes and their descendants.

---

## 2. Components and instances

Sources: https://help.figma.com/hc/en-us/articles/360038662654 (Guide to components), https://help.figma.com/hc/en-us/articles/360039150733 (Apply overrides to instances), https://help.figma.com/hc/en-us/articles/360056440594 (Variants), API `ComponentNode`, `InstanceNode.mainComponent`, `InstanceNode.overrides`, `detachInstance()`, `resetOverrides()`, `swapComponent()`.

### Rules
* **Main component** (`COMPONENT`) is a frame-like node and the source of truth. Create it with `Cmd+Alt+K`. On a single frame or group the node is converted in place. On a multi-selection, the default creates one component wrapping all of them; the dropdown can instead create *multiple* components, one per layer.
* **Instance** (`INSTANCE`) is a frame-like node whose subtree mirrors the main. Create one by copy/pasting the main, Alt-dragging the main, or dragging from the assets panel. Every instance child has a stable id that maps it to the corresponding main child (Figma uses `I<instanceId>;<mainChildId>` composite ids).
* **Structure is locked in instances:** you cannot add, delete or reorder children, and you cannot change a child's position, rotation or constraints. You can resize the **instance itself**: its children respond via constraints or auto layout. Inside an auto-layout instance, children can change sizing and visibility.
* **Overridable per child (each property tracked independently):** visibility; fills; strokes (paint, weight, align, dash); effects; layer opacity and blend mode; corner radius; text characters and all text styling (per-range); font; layout grids; export settings; nested-instance **swap**; component property values (text, boolean, instance-swap, variant); auto-layout padding/gap/sizing on instance sublayers; layer name. The instance root can additionally override size and its own frame props.
* **Propagation:** an edit to the main writes property P on main-child C. For every instance, if (instance-child ↔ C) has **no override for P**, the instance shows the new value. If it has an override, the instance keeps it. Overrides are per *field*, not per node: overriding a text's color does not freeze its characters.
* Structural edits on the main (add, remove, reorder, move, resize) always propagate. New children appear in all instances. Deleting a main child removes it from instances along with its overrides.
* Editing an instance property to a value **equal to the main's value** still records an override in Figma (verify). Sketch may choose to drop equal overrides.
* **Reset overrides:** "Reset all changes" clears all overrides on the instance subtree. Per-property reset is in the context menu of each panel section.
* **Detach instance** `Cmd+Alt+B`: converts the instance into a plain frame with the *resolved* values (main + overrides baked in). Nested instances stay instances.
* **Swap instance:** replace `mainComponent`. Overrides are preserved for children whose **layer name and hierarchy path** match in the new component. Others are dropped. The size is kept if the instance was resized, otherwise it takes the new main's size (verify).
* **Go to main component**: context menu. **Push changes to main** (overrides → main) and "Reset" exist in the right-panel instance section.
* **Deleting the main** leaves instances functional. They reference a missing main and can "restore" it.
* **Variants (brief):** a `COMPONENT_SET` frame contains components named `Prop1=Value, Prop2=Value`. An instance exposes each property as a dropdown, and switching variant = swap to the sibling component with matching values, preserving overrides by name/path. Other component property kinds: **Boolean** (binds to a layer's visibility), **Text** (binds to a text layer's characters), **Instance swap** (binds to a nested instance's main), **Variant**.

### Override resolution pseudo-code
```
resolve(instance_child, prop):
  if instance_child.overrides.contains(prop): return override value
  main_child = instance_child.main_ref
  if main_child is itself inside an instance (nested): return resolve(main_child, prop)
  return main_child.prop
```

**Sketch note:** store instances as `{ main: NodeId, overrides: HashMap<(MainChildPath), PropOverrides> }` and **materialize** the instance subtree lazily (a render/hit cache). Do not deep-copy nodes on instance creation. Use a `PropOverrides` struct of `Option<T>` fields so per-field propagation falls out naturally.

---

## 3. Auto layout

Sources: https://help.figma.com/hc/en-us/articles/360040451373 (Explore auto layout properties), https://help.figma.com/hc/en-us/articles/5731482952599 (Wrap / gap), API `layoutMode`, `itemSpacing`, `counterAxisSpacing`, `primaryAxisAlignItems`, `counterAxisAlignItems`, `counterAxisAlignContent`, `layoutWrap`, `paddingLeft/Right/Top/Bottom`, `layoutSizingHorizontal/Vertical` (https://developers.figma.com/docs/plugins/api/properties/nodes-layoutsizinghorizontal/), `layoutGrow`, `layoutAlign`, `layoutPositioning`, `strokesIncludedInLayout`, `itemReverseZIndex`, `minWidth/maxWidth/minHeight/maxHeight`.

### Properties and defaults
| Property | Values | Default |
|---|---|---|
| `layoutMode` | NONE, HORIZONTAL, VERTICAL, GRID (new) | NONE. `Shift+A` on a selection infers the direction from the children's arrangement: if they are spread more along x than y it uses HORIZONTAL, else VERTICAL. |
| `layoutWrap` | NO_WRAP, WRAP | NO_WRAP. Docs say wrap is available for horizontal and vertical flows. Historically it was horizontal only. |
| `itemSpacing` (gap) | number (may be **negative**) or "Auto" | `Shift+A` on existing layers infers the gap from the current average spacing, rounded. An empty frame gets 10 in the UI. The API default is 0. |
| Auto gap | Between (= space-between). Newer versions add Around and Evenly. | Toggle with `X` in the alignment widget |
| `counterAxisSpacing` | row gap when wrapping, number | = itemSpacing initially |
| `counterAxisAlignContent` | AUTO, SPACE_BETWEEN (wrap rows) | AUTO |
| padding L/R/T/B | numbers ≥ 0 | Inferred from existing offsets on `Shift+A`. Empty frame gets 10 (UI) or 0 (API). |
| `primaryAxisAlignItems` | MIN, CENTER, MAX, SPACE_BETWEEN | MIN |
| `counterAxisAlignItems` | MIN, CENTER, MAX, BASELINE (horizontal only) | MIN |
| sizing per axis | FIXED, HUG, FILL | Frame: HUG on both after `Shift+A` on a selection, FIXED on a pre-sized frame. Child: FIXED (text: HUG). |
| `layoutPositioning` | AUTO, ABSOLUTE ("Ignore auto layout") | AUTO |
| `strokesIncludedInLayout` | bool ("Include strokes" in layout) | false: strokes do not occupy space |
| `itemReverseZIndex` | bool ("Canvas stacking: first on top") | false: the last child paints on top |
| min/max W/H | Option<number> | none |

### Sizing validity (from API doc, quoted)
* "`HUG` is only valid on auto-layout frames and text nodes."
* "`FILL` is only valid on auto-layout children."
* Manually resizing a layer, or typing W/H, switches that axis to **FIXED**.
* **Hug parent + Fill child on the same axis:** the docs say "if any child is set to Fill container, the parent no longer hugs on that axis and becomes Fixed". Apply this on the **primary** axis. On the **counter** axis Figma allows hug-parent with fill children *if at least one child is non-fill*: the parent hugs the largest non-fill child and fill children stretch to it. If all children are fill on the counter axis, the parent flips to FIXED (verify the exact counter-axis rule).
* Fill on the primary axis = `layoutGrow = 1`. All fill children split the remaining space **equally** (Figma grow factors are only 0 or 1), clamped by min/max with redistribution.
* Fill on the counter axis = `layoutAlign = STRETCH`: child size = parent inner size on that axis.
* Text in auto layout: HUG width = auto-width text, FIXED width + HUG height = auto-height, FILL width implies auto-height text (wraps).
* Children of auto layout have **no constraints** unless ABSOLUTE. ABSOLUTE children use normal constraints relative to the parent (§4) and are ignored by flow, sizing and hug.
* **Hidden children (`visible=false`) are skipped entirely**: no size, no gap slot.
* Shortcuts: double-click a frame edge = hug that axis. Alt+double-click = fill. Alignment: arrows (in the 3×3 widget) or `W/A/S/D`. Baseline: `B`. Padding fields: Cmd-click edits all sides, Alt-click edits opposite sides. Ctrl-drag a child in to make it absolute.
* **Reorder by drag:** dragging a flow child inside an auto-layout frame shows an insertion bar. Order = layer order (first child = left/top). Dropping a layer from outside inserts it at the bar.
* `Shift+A` adds auto layout (on a frame: converts it; on a non-frame selection: wraps it in a new auto-layout frame). Removing it is `Alt+Shift+A` (verify). When removing, children keep their absolute positions.

### Layout algorithm (one frame, single pass after children are measured bottom-up)
```
layout(F):                                   # F has layoutMode != NONE
  P = primary axis (x for HORIZONTAL), C = counter axis
  kids = [c in F.children if c.visible and c.positioning == AUTO]
  sw = F.strokesIncludedInLayout ? F.strokeWeight (inside/center adjusted) : 0
  pad_p0, pad_p1, pad_c0, pad_c1 = paddings (+ sw if included)

  # 1. measure children (recursive: auto-layout children lay out first if they HUG)
  for k in kids:
      k.size_p = (k.sizing_p == HUG) ? intrinsic_p(k) : (k.sizing_p == FIXED ? k.w_p : 0)
      k.size_c = (k.sizing_c == HUG) ? intrinsic_c(k) : (k.sizing_c == FIXED ? k.w_c : 0)
      k.outer  = size + (k.strokesInLayout ? stroke outset : 0)   # only if F includes strokes

  if not F.wrap:
      gap = F.gap_auto ? 0 : F.itemSpacing
      fixed_sum = sum(k.size_p for k in kids if k.sizing_p != FILL) + gap*(len(kids)-1)

      # 2. resolve F's primary size
      if F.sizing_p == HUG:   F.size_p = clamp(pad_p0 + fixed_sum + pad_p1, F.min_p, F.max_p)
      inner_p = F.size_p - pad_p0 - pad_p1

      # 3. distribute fill along primary (equal shares, honor min/max with re-loop)
      fills = [k for k in kids if k.sizing_p == FILL]
      remaining = inner_p - fixed_sum
      repeat until stable:
          share = remaining / len(fills)
          for k in fills: k.size_p = clamp(share, k.min_p, k.max_p)
          freeze clamped ones, subtract them from remaining, recompute share for the rest

      # 4. counter size
      if F.sizing_c == HUG:
          F.size_c = clamp(pad_c0 + max(k.size_c for k in kids if k.sizing_c != FILL, default 0) + pad_c1, ...)
      inner_c = F.size_c - pad_c0 - pad_c1
      for k in kids if k.sizing_c == FILL: k.size_c = clamp(inner_c, k.min_c, k.max_c)

      # 5. place along primary
      used = sum(k.size_p) + gap*(n-1)
      free = inner_p - used
      if F.gap_auto (SPACE_BETWEEN):
          gap = n > 1 ? max(0, (inner_p - sum(k.size_p)) / (n-1)) : 0
          start = pad_p0                         # single child sits at start (verify)
      else switch primaryAlign:
          MIN: start = pad_p0; CENTER: start = pad_p0 + free/2; MAX: start = pad_p0 + free
      cursor = start
      for k in kids (layer order; RTL not supported):
          k.pos_p = cursor; cursor += k.size_p + gap
          # 6. counter alignment
          switch counterAlign:
              MIN: k.pos_c = pad_c0
              CENTER: k.pos_c = pad_c0 + (inner_c - k.size_c)/2
              MAX: k.pos_c = pad_c0 + inner_c - k.size_c
              BASELINE: k.pos_c = pad_c0 + (maxBaseline - k.firstBaseline)
      # overflow: negative free is allowed; children overflow and clip only if F.clipsContent
  else: # WRAP (horizontal): greedy line breaking
      lines = []; line = []; x = 0
      for k in kids:  # FILL on primary is not allowed in wrap rows (treated as FIXED; verify)
          if line and x + gap + k.size_p > inner_p: lines.push(line); line=[]; x=0
          x += (line ? gap : 0) + k.size_p; line.push(k)
      lines.push(line)
      each line: lay out as the non-wrap case on primary (incl. alignment / space-between per line)
      line_h = max(k.size_c in line)
      stack lines on C with counterAxisSpacing (or space-between if counterAxisAlignContent == SPACE_BETWEEN)
      counterAlign aligns each item within its line height
      if F.sizing_c == HUG: F.size_c = pad_c0 + sum(line_h) + rowgap*(L-1) + pad_c1

  # 7. absolute children: apply constraints (§4) against F's old→new size
  for k in F.children if k.positioning == ABSOLUTE: apply_constraints(k, F.old_size, F.size)

  pixel snapping: Figma keeps fractional positions; the UI rounds displayed values to 2 dp
```
Order of evaluation: HUG sizes are bottom-up (child before parent) and FILL sizes are top-down (parent before child). Two passes are enough: (1) a post-order measure of intrinsic HUG sizes treating FILL as 0 / min, then (2) a pre-order arrange that assigns FILL sizes and positions and re-lays out any child whose size changed.

**Sketch note:** this maps 1:1 to CSS flexbox (`flex-direction`, `gap`, `padding`, `justify-content`, `align-items`, `flex-wrap`, `flex: 1 1 0` for fill, `align-self: stretch`, `position:absolute`), which is also Paper's model (§14). Implement it as `layout::flex(frame)` in a separate module run after each transaction on dirty subtrees. Store `sizing: {h: Fixed|Hug|Fill, v: …}` per node.

---

## 4. Constraints

Sources: https://help.figma.com/hc/en-us/articles/360039957734 (Apply constraints), API `constraints: {horizontal, vertical}` with `MIN | CENTER | MAX | STRETCH | SCALE`.

### Rules
* Constraints live on **children of frames** (not on group children, and not on auto-layout flow children). They are evaluated **only when the parent frame is resized** (not when it moves). The default is **Left + Top** (`MIN`,`MIN`).
* Horizontal: Left (MIN), Right (MAX), Left & Right (STRETCH), Center, Scale. Vertical: Top, Bottom, Top & Bottom, Center, Scale.
* Groups: a group's children are positioned by the group's *own* constraints relative to the group's parent frame. Resizing a group scales its children proportionally, like SCALE.
* Hold **Cmd while resizing** a frame to ignore constraints. Resizing an instance always applies constraints.
* UI: the constraint widget picks with click, and Shift-click adds the opposite side (→ STRETCH).
* Constraints are relative to the **parent frame's bounds** (not padding). Layout grids can scope constraints to a column (skip for Sketch).
* Rotated children: constraints apply to the child's axis-aligned bounding box (verify).

### Pseudo-code (per axis; shown for x)
```
apply_constraint_x(child, old_w, new_w):     # positions relative to parent origin
  l = child.x; r = old_w - (child.x + child.w); cx_off = (child.x + child.w/2) - old_w/2
  switch child.constraints.h:
    MIN:     x' = l;                          w' = child.w
    MAX:     x' = new_w - r - child.w;        w' = child.w
    STRETCH: x' = l;                          w' = max(new_w - l - r, 0)   # Figma allows clamping to ≥ 0.01
    CENTER:  x' = new_w/2 + cx_off - child.w/2; w' = child.w
    SCALE:   s = new_w / old_w; x' = l*s;     w' = child.w*s
  if w' != child.w and child is frame: recurse apply_constraints on its children (old w → w')
  if child is text and w changes: text reflows (auto-height text grows vertically)
```
* Drag-resize from the left/top edge also moves the parent's origin. Express everything relative to the parent so the child's absolute position shifts with MIN constraints exactly as the user expects (children "stick" to the left edge, which moved).
* To avoid drift from repeated resizes, compute from the state captured at **drag start** (the original child rects), not incrementally per mouse move.

**Sketch note:** add `Constraints { h: HConstraint, v: VConstraint }` to `Node` (default Min/Min). In the resize gesture, snapshot the child rects on pointer-down and recompute from the snapshot each frame. Cmd during the resize sets `ignore_constraints`.

---

## 5. Vector networks, pen tool, booleans, flatten

Sources: https://help.figma.com/hc/en-us/articles/360040450213 (Vector networks), https://help.figma.com/hc/en-us/articles/360039957534 (Boolean operations), API `VectorNode.vectorNetwork` (`vertices`, `segments` with `tangentStart/End`, `regions` with `windingRule`), `BooleanOperationNode.booleanOperation`, `figma.flatten()`. Paper: https://paper.design/docs/svg.

### Vector network model
* A vector network is a **graph**, not a list of paths: `vertices[]` (x, y, `strokeCap`, `cornerRadius`, `handleMirroring: NONE | ANGLE | ANGLE_AND_LENGTH`), `segments[]` (start, end vertex index, `tangentStart`, `tangentEnd` relative vectors), and `regions[]` (loops of segment indices + `windingRule` NONZERO/EVENODD + per-region fills). A vertex may join **3+ segments**, which SVG paths cannot express.
* Export to SVG decomposes the network into paths. Regions become filled subpaths, and the stroke follows every segment.

### Pen tool (`P`)
* **Click** places a **corner** vertex (zero tangents) and connects a segment from the previous vertex.
* **Click-drag** places a **smooth** vertex: the outgoing handle follows the pointer and the incoming handle is **mirrored (angle and length)**. Mirroring is `ANGLE_AND_LENGTH`.
* **Alt while dragging** (or Alt-drag an existing handle) breaks mirroring so only the dragged handle moves (`NONE`). In Figma, Alt-dragging during creation sets the outgoing handle independently (verify the exact modifier timing).
* **Shift** constrains the segment or handle angle to 45° increments.
* **Closing:** hovering the **first vertex** shows a close indicator, and clicking it closes the loop and creates a fillable region. Clicking any *other* existing vertex connects to it (a network branch) and continues drawing from it.
* **Finish an open path:** `Enter` or `Esc` (Figma: Esc/Enter end the current path but stay in vector edit mode; a second Esc/Enter exits to object selection). Double-clicking the last point also ends the path (verify).
* **Edit mode:** double-click a vector (or press Enter with it selected) to enter. Exit with Esc/Enter or by clicking outside. In edit mode: click selects a vertex, Shift-click multi-selects, drag moves it, and marquee selects vertices. **Cmd (hold)** = bend tool: drag a segment to curve it, and click a vertex to toggle corner/smooth. `Delete` removes a vertex and heals the path where possible. Deleting a segment splits the path. `B` = paint bucket to toggle region fills (verify).
* Pencil `Shift+P`: freehand stroke, simplified to curves on release.
* Paper (docs/svg): Pen `P` adds segments. Clicking inside an existing SVG adds a path to it, clicking outside creates a new SVG. "Move tool (M)" adjusts points. Enter enters path editing and esc exits (double-click also enters, double-click empty space exits). **Pixel snapping** snaps path points to the nearest **half-pixel**, toggled with `Cmd+Shift+'`. Booleans are on Paper's roadmap and not shipped.

### Boolean operations (official help, quoted semantics)
| Op | Shortcut (Mac) | Geometry | Style source |
|---|---|---|---|
| Union | `Alt+Shift+U` | area covered by any operand | **top** layer's fill/stroke/effects. Stroke on outer path. |
| Subtract | `Alt+Shift+S` | **bottom-most layer minus all layers above it** | **bottom** layer. Strokes along both inner and outer edges. |
| Intersect | `Alt+Shift+I` | area covered by **all** operands | **top** layer. Non-overlapping operands vanish from view. |
| Exclude | `Alt+Shift+E` | area covered by an **odd** number of operands (XOR) | **top** layer. Strokes along both edges. |
* The result is a **non-destructive** `BOOLEAN_OPERATION` group. Children stay editable in size, position, rotation and corner radius, but their individual fill/stroke/effects/opacity are ignored. Change the op type from the toolbar/dropdown. Ungroup restores the children. Booleans can nest, and the result is evaluated bottom-up.
* Operands are evaluated with their fill geometry (strokes are not part of the shape). Order = layer order, bottom first.
* **Flatten** `Cmd+E`: bakes the selection (booleans, shapes, text) into a single vector network, destructively. Style comes from the top or the boolean group. **Outline stroke** `Cmd+Alt+O` converts a stroke into a filled vector.

**Sketch note:** use `lyon` for tessellation, and a polygon clipper (`i_overlay` or `geo` boolean, both clean-room, MIT/Apache) on flattened curves for boolean evaluation, cached per boolean node. Start with a path-list model where vertices have `handle_in/handle_out/mirroring`. A full branching network is optional.

---

## 6. Fills, strokes, effects, opacity, blend

Sources: https://help.figma.com/hc/en-us/articles/360041003694 (Fills), https://help.figma.com/hc/en-us/articles/360049283914 (Strokes), https://help.figma.com/hc/en-us/articles/360040028114 (Image fills), API `Paint` (`SolidPaint`, `GradientPaint` with `gradientTransform` + `gradientStops`, `ImagePaint.scaleMode` FILL|FIT|CROP|TILE), `strokeAlign`, `dashPattern`, `strokeCap`, `strokeJoin`, `strokeMiterLimit`, `topLeftRadius…`, `cornerSmoothing`, `BlendMode`.

* **Multiple fills** form a stack drawn **bottom → top**, with the panel showing top first. Each paint has `visible`, `opacity` (0–1) and `blendMode`. Add with "+", which duplicates the top fill's color (verify) or uses a default: shapes get `#D9D9D9`, frames `#FFFFFF`, text `#000000`.
* **Solid:** RGB + paint opacity.
* **Gradients:** LINEAR, RADIAL, ANGULAR (conic), DIAMOND. Stops are `{position 0..1, color RGBA}`, with at least 2. Geometry is a 2×3 `gradientTransform` in the node's unit space. The UI shows 3 handles: start, end, and a width handle for radial/diamond (ellipse radii). Click on the gradient line to add a stop, drag stops, Delete removes the selected stop (min 2). The default linear runs top → bottom, center x, from color@100% to color@0% (verify).
* **Image fill scale modes:** **FILL** (cover: scale to fill the bounds, crop overflow, centered), **FIT** (contain: letterbox, centered), **CROP** (user-defined transform/crop rect, stretches with the layer), **TILE** (repeat at `scalingFactor`, origin top-left). Plus image filters (exposure, contrast, saturation, temperature, tint, highlights, shadows) and rotate by 90°.
* **Strokes:** a paint stack like fills, plus a single `strokeWeight` (per-side weights on frames/rects: top/right/bottom/left). `strokeAlign`: **INSIDE** (default for frames/rects/ellipses/closed shapes), **CENTER** (default for open vectors/lines; the only option for open paths), **OUTSIDE**. Inside/outside are implemented as a 2× wide center stroke clipped to inside or outside the fill geometry.
* **Dash:** `dashPattern: [dash, gap, …]` in px. The UI exposes Dash and Gap (gap defaults to the dash value). Caps apply per dash.
* **Caps** (open paths): NONE, ROUND, SQUARE, plus arrow/line/triangle/circle/diamond end markers (per endpoint). **Joins:** MITER (default, limit ≈ 28.96° angle in Figma terms), BEVEL, ROUND.
* **Corner radius:** uniform or **per corner** (`topLeftRadius`, `topRightRadius`, `bottomRightRadius`, `bottomLeftRadius`). Radius is clamped to ½ of min(w,h), and with adjacent large radii it is scaled proportionally like CSS. `cornerSmoothing` 0–1 (iOS squircle, 0.6 ≈ iOS). Vector vertices have per-vertex radius. Dragging the on-canvas radius handle edits all corners, and Alt-drag edits one corner.
* **Layer opacity vs fill opacity:** layer opacity (`node.opacity`, keys **0–9**: `1`=10% … `0`=100%, two quick digits like `5`,`5` = 55%) composites the **whole layer** (fills+strokes+children) as one group, so overlaps do not double-darken. Fill opacity is per paint and multiplies only that paint.
* **Blend modes** (layer and per-paint): PASS_THROUGH (groups/frames default for layer), NORMAL, DARKEN, MULTIPLY, COLOR_BURN, LIGHTEN, SCREEN, COLOR_DODGE, OVERLAY, SOFT_LIGHT, HARD_LIGHT, DIFFERENCE, EXCLUSION, HUE, SATURATION, COLOR, LUMINOSITY (plus PLUS_DARKER/LINEAR_BURN, PLUS_LIGHTER/LINEAR_DODGE). These match CSS `mix-blend-mode` except pass-through.
* **Effects (brief):** drop shadow (default x 0, y 4, blur 4, spread 0, `#000` 25%), inner shadow, layer blur, background blur. Shadows follow the fill shape. "Show behind transparent areas" is a toggle.

**Sketch note:** model `fills: Vec<Paint>`, `strokes: Vec<Paint>`, `stroke: StrokeStyle {weight, align, dash, cap, join, miter}`, `corner_radii: [f32;4]`. Render layer opacity < 1 or non-normal blend through an offscreen layer (wgpu render-to-texture), and render per-paint opacity inline. Implement inside/outside stroke via stencil or clip of a 2× stroke.

---

## 7. Text

Sources: https://help.figma.com/hc/en-us/articles/360039956434 (Text properties), https://help.figma.com/hc/en-us/articles/360039957034 (Text styles), API `TextNode.textAutoResize` (NONE | WIDTH_AND_HEIGHT | HEIGHT | TRUNCATE), `getStyledTextSegments`, `setRangeFontSize/...`, `lineHeight {unit: PIXELS|PERCENT|AUTO}`, `letterSpacing {unit: PIXELS|PERCENT}`, `textAlignHorizontal` LEFT|CENTER|RIGHT|JUSTIFIED, `textAlignVertical` TOP|CENTER|BOTTOM, `paragraphSpacing`, `textCase`, `textDecoration`, `textTruncation`, `maxLines`.

* **Creation:** `T` then **click** creates **Auto width** (WIDTH_AND_HEIGHT: one line per paragraph, grows rightwards). **Click-drag** creates **Fixed size** (NONE), drawn as a box. Typing and then clicking away with empty content deletes the layer.
* **Resize modes:** Auto width (W and H hug the content; no wrapping except at hard newlines). **Auto height** (fixed W, wraps, H hugs). **Fixed** (W and H fixed, overflow visible, not clipped). **Truncate** (fixed with an ellipsis, plus `maxLines`). Dragging a side handle of auto-width text converts it to auto height. Dragging the top/bottom handle converts it to fixed. Double-clicking the right handle reverts to auto width (verify).
* Alignment: horizontal L/C/R/Justified, vertical Top/Middle/Bottom (meaningful in Fixed). Shortcuts: `Cmd+Alt+L/T/R/J` left/center/right/justify.
* **Per-range styles:** any character range may have its own font family, weight/style, size, fill paints, letter spacing, line height, decoration, case and link. Paragraph-level properties (alignment, paragraph spacing, list style, indent) apply per paragraph. Panels show "Mixed" for multi-value selections. Typing at a caret inherits the style of the **preceding character** (at the start of a paragraph, the following character).
* Shortcuts in edit mode: `Cmd+B` bold (switches to the family's Bold weight if present), `Cmd+I` italic, `Cmd+U` underline, `Cmd+Shift+X` strikethrough, `Cmd+Shift+>`/`<` font size ±1 (Alt makes it ±10). Line-height and letter-spacing nudges also exist as Alt-based shortcuts (verify the exact keys before binding them).
* **Line height:** Auto (font metrics, roughly 1.2× size for most fonts), px, or %. Text is vertically centered within the line box (half-leading like CSS). The first baseline = top + (lineHeight − (ascent+descent))/2 + ascent.
* **Letter spacing:** px or % of font size. The default is 0%. It is applied after every character, including the last (verify; CSS also adds trailing spacing).
* **Paragraph spacing** in px is added between paragraphs. Paragraph indent applies to the first line.
* Defaults: **Inter, Regular, 12px** (verify the current UI3 default), line height Auto, letter spacing 0%, fill `#000000`.
* **Text styles:** a named style = {font family, style, size, line height, letter spacing, paragraph spacing, case, decoration}. Applying one links the range. Editing the style updates all linked ranges. Detaching keeps the values. Fill color is *not* part of a text style (a color style is separate).
* Scaling text with the Scale tool `K` (or Shift+drag with K) scales the font size. A normal resize only reflows.

**Sketch note:** current `render/text.rs` lays out glyph outlines with `ttf-parser`. Add `TextAutoResize` and an attributed-string model `Vec<(Range<usize>, TextStyle)>` merged on edit. Ship Inter (OFL) as already planned and keep egui's font as fallback.

---

## 8. Alignment, distribution, tidy up

Sources: https://help.figma.com/hc/en-us/articles/360039956914 (Adjust alignment, rotation, and position), https://help.figma.com/hc/en-us/articles/360040450233 (Smart selection / tidy up).

* **Reference box:**
  * ≥2 layers selected: align to the **selection's union bounds** (rotated layers use their AABB).
  * Exactly 1 layer selected **inside a frame**: align to the **parent frame's bounds** (not padding). On a top-level layer with no parent frame, align is disabled/no-op.
  * Within auto-layout flow children, align/distribute are disabled (layout owns position).
  * Key object: none in Figma (Illustrator-style key object is not supported). All layers move.
* **Align left/hcenter/right/top/vcenter/bottom** (`Alt+A`, `Alt+H`, `Alt+D`, `Alt+W`, `Alt+V`, `Alt+S`): set each layer's AABB edge or center to the reference's. Results may be fractional, and Figma does not round (verify).
* **Distribute horizontal/vertical spacing** (`Alt+Shift+H`, `Alt+Shift+V`; needs ≥3): sort by AABB min on the axis. Keep the first and last fixed. gap = (span − Σsizes)/(n−1). Place sequentially. Gaps may be negative when layers overlap.
* **Distribute centers** (Figma "distribute left/centers" in the dropdown, older): equal spacing between centers.
* **Tidy up** (`Ctrl+Alt+T`): for a selection that roughly forms a row, column or grid, snap it to an even grid. Figma clusters into rows and columns by overlap, uses the **median/most-common** spacing (rounded) as the gap, aligns each row's top edge and each column's left edge, and keeps the first item's position. The result becomes a "smart selection" (§9).
* **Smart selection:** when the selected items are evenly spaced, the canvas shows pink spacing handles between them. Dragging a handle changes all gaps together, and dragging an item's center dot reorders it (swaps positions).

**Sketch note:** implement `align(nodes, Edge, reference)` and `distribute(nodes, axis)` as pure functions over AABBs returning translations, then wrap them in one undo transaction.

---

## 9. Smart guides, snapping and measurement

Sources: https://help.figma.com/hc/en-us/articles/360040450233, https://help.figma.com/hc/en-us/articles/360039956974 (Measure distances), plus observed behavior.

* **Snap targets** while moving or resizing (in document space): the left/center/right x and top/center/bottom y of (a) siblings in the same parent, (b) the parent frame, (c) other top-level frames when at top level, (d) layout grid lines and (e) the pixel grid when "Snap to pixel grid" is on (default on: positions rounded to integers at the end of a move). The dragged layer's own 3 x-anchors and 3 y-anchors are tested against all targets.
* **Threshold:** roughly **4–6 screen px** (Figma feels like ~5). It is converted to document units as `thr / zoom`. Sketch already uses `SNAP_PX = 6.0` screen px (`src/editor/snap.rs`), which is fine. Per axis, the target with the smallest |delta| wins, and x and y snap independently.
* **Guides:** red/pink (`#F24822`-ish "Figma red") 1 px lines spanning from the dragged layer to the matched target, with small × markers at the matched points. Multiple targets with the same coordinate are all drawn.
* **Equal spacing:** when the gap between the dragged layer and a neighbor equals an existing gap between two other siblings on the same axis (within the threshold), snap to it and draw spacing indicators (pink bars with the px value) on **all** equal gaps.
* **Disable snapping:** Figma has no single documented modifier. Cmd while *resizing* means "ignore constraints", not "no snap". Sketch should hold **Ctrl** (or offer a toggle) to suspend snapping (verify against Figma). Nudging with the arrow keys (1 px, Shift = 10 px, configurable "big nudge") never snaps.
* **Alt-hover measurement:** with a layer selected, holding **Alt** and hovering another layer shows red lines with **distance labels** between the nearest edges of the selection and the hovered layer on each axis (only axes with a gap). Hovering the parent (or empty space inside the parent) shows the distances from the selection to the parent's 4 inner edges. Overlapping layers show inside distances (edge to edge). Labels are white text on red rounded pills, values rounded to 2 dp and displayed without trailing zeros.
* **Size label:** while drawing, moving or resizing, a blue/primary pill under the selection shows `W × H`. While moving, Figma shows the distance to the parent edges only with Alt.
* Rotation snaps to 15° with Shift. Resizing with Shift keeps the aspect ratio, and Alt resizes from the center.

**Sketch note:** `editor/snap.rs` already does edge/center snapping. Add (1) equal-spacing detection: collect sibling gaps on each axis and match the candidate gap within the threshold, (2) an Alt-measure overlay computed on hover and drawn in screen space, (3) a pixel-grid rounding toggle. Keep all thresholds in **screen px**.

---

## 10. Layers panel

Sources: https://help.figma.com/hc/en-us/articles/360039959014 (Layers panel), observed.

* The tree shows **page children**, with the top-most z first (reverse of paint order). Indentation is roughly **16 px per depth** (UI3 rows are 32 px tall; older UI 28 px). Icons show type (frame #, group dashed square, component, instance, text T, vector pen, image, boolean). Auto-layout frames get direction icons.
* Expand/collapse via the disclosure caret. **Alt+click a caret** toggles all descendants. `Alt+L` collapses all layers. Selecting on canvas auto-expands ancestors and scrolls the row into view.
* **Selection:** click selects. Shift-click selects a range (between the anchor and the clicked row, same as list semantics, across depths in visual order). Cmd-click toggles.
* **Rename:** double-click the name, or `Cmd+R` (with multiple selected it opens batch rename with find/replace and `$&`/`$n` numbering). Enter commits, Esc cancels, and Tab moves to the next layer's rename. An empty name reverts to the default.
* **Hover affordances:** an eye (visibility) and a lock icon appear on hover at the right of the row and stay visible while toggled (hidden/locked). Hidden rows are dimmed. Clicking the eye toggles; drag across eyes toggles many (verify). Shortcuts: `Cmd+Shift+H` show/hide, `Cmd+Shift+L` lock/unlock.
* **Drag reorder:** dragging rows shows a horizontal **insertion line** between rows. The line's **indent depth** chooses the parent: moving the pointer left or right while over the boundary picks shallower or deeper levels, following the tree semantics of "after the last child of X at this depth". Dropping onto the **middle** of a frame or group row highlights the row and inserts as its **top-most child**. Dropping into a collapsed container also inserts at the top. Absolute canvas position is **preserved** on reparent (the node's local transform is recomputed). Exception: dropping into auto layout places the node in the flow at the insertion index.
* Invalid drops: into its own descendants, into a non-container (shape/text), or into instance sublayers (structure is locked). Show no indicator and treat as a no-op.
* Multi-row drag keeps the relative order of the dragged nodes.
* Selected rows use a light primary-tint background. Rows whose ancestor is selected get a lighter tint.

**Sketch note:** the egui tree needs a drop-target resolver: `(row_index, pointer_y_fraction, pointer_x) → DropTarget::{Before(id), After(id, depth), Into(id)}` with the top 25% = before, bottom 25% = after, and the middle 50% = into for containers. Preserve world transforms on reparent.

---

## 11. Pages

Sources: https://help.figma.com/hc/en-us/articles/360039958594 (Pages), API `PageNode`, `figma.currentPage`.

* Each file has an **ordered list of pages**. Each page is an independent infinite canvas with its own background color (default `#F5F5F5` light / `#1E1E1E` dark, verify), its own selection and its **own viewport** (zoom and scroll restored when switching back). Prototype flows and guides are per page.
* The page list is at the top of the layers panel (collapsible). "+" adds "Page N" after the last page and switches to it. Double-click renames inline. Drag reorders. Context menu: Rename, Duplicate (adds "<name> copy" right after, deep-copying all nodes with new ids), Delete (disabled for the last remaining page; deleting moves to the adjacent page), Copy link.
* Page dividers: a page named `---` (or any name with only dashes) renders as a separator (Figma feature, optional).
* **Copy/paste across pages:** `Cmd+C` then switching page and `Cmd+V` pastes at the **same document coordinates** as the source *if* those coordinates are visible in the viewport. Otherwise it pastes centered in the viewport (verify; this is also the rule within a page). `Cmd+Shift+V` = **paste over selection / paste in place**, which always keeps the original coordinates. Pasting with a frame selected pastes *into* that frame (same relative offset if it fits, else centered).
* "Move to page" in the context menu moves layers to another page, keeping coordinates.
* Undo history is per file (spans pages), and an undo switches to the affected page.

**Sketch note:** `Document { pages: Vec<Page> }`, `Page { id, name, roots: Vec<NodeId>, bg, camera }`. The node arena can stay global. Persist `camera` per page in the `.sketch-rs` file.

---

## 12. Command palette and keyboard shortcuts

Sources: https://help.figma.com/hc/en-us/articles/360040328653 (Keyboard shortcuts), https://help.figma.com/hc/en-us/articles/23570416033943 (Actions menu), forum confirmations (https://forum.figma.com/t/in-ui3-return-to-cmd-k-shortcut-for-adding-links/78622). In-app: `Ctrl+Shift+?` opens the shortcut panel.

### Command palette
* **Figma UI3: `Cmd+K` opens the "Actions" menu** (search commands, plugins, components, and recent actions). **`Cmd+/` (Windows `Ctrl+/` or `Ctrl+P`) is the legacy "Quick actions"** and still opens the same menu. Create link moved from Cmd+K to `Cmd+Shift+U`.
* Behavior: a fuzzy search over menu commands (with their shortcut shown), plugins, assets and settings. Enter runs, ↑/↓ navigate, Esc closes. Recent commands show first when the query is empty. Commands that are disabled for the current selection are hidden or greyed.
* **Sketch:** bind both `Cmd+K` and `Cmd+/` to a palette fed by the single keymap table (`editor/shortcuts.rs`), so the palette, menus and README stay in sync.

### Figma shortcut table (macOS; Windows: Cmd→Ctrl, Alt=Alt)
| Category | Action | Shortcut |
|---|---|---|
| Tools | Move / Scale | `V` / `K` |
| | Frame | `F` or `A` |
| | Section | `Shift+S` |
| | Rectangle / Ellipse / Line / Arrow | `R` / `O` / `L` / `Shift+L` |
| | Pen / Pencil | `P` / `Shift+P` |
| | Text | `T` |
| | Hand (temporary) | hold `Space` (also `H` in UI3) |
| | Comment | `C` |
| | Place image | `Cmd+Shift+K` |
| | Slice | `S` |
| | Color picker (eyedropper) | `I` / `Ctrl+C` |
| Selection | Select all / inverse | `Cmd+A` / `Cmd+Shift+A` |
| | Select children / parent | `Enter` / `Shift+Enter` or `\` |
| | Next / previous sibling | `Tab` / `Shift+Tab` |
| | Deep select | `Cmd`+click |
| | Select matching layers | `Cmd+Alt+A` |
| Edit | Undo / Redo | `Cmd+Z` / `Cmd+Shift+Z` |
| | Copy / Cut / Paste | `Cmd+C` / `Cmd+X` / `Cmd+V` |
| | Paste over selection / in place | `Cmd+Shift+V` |
| | Paste to replace | `Cmd+Shift+R` |
| | Duplicate | `Cmd+D` (or Alt-drag) |
| | Delete | `Delete` / `Backspace` |
| | Copy / paste properties | `Cmd+Alt+C` / `Cmd+Alt+V` |
| | Copy as PNG | `Cmd+Shift+C` |
| | Rename | `Cmd+R` |
| | Find | `Cmd+F` |
| | Flatten | `Cmd+E` |
| | Outline stroke | `Cmd+Alt+O` |
| | Use as mask | `Cmd+Ctrl+M` |
| | Flip horizontal / vertical | `Shift+H` / `Shift+V` |
| | Nudge / big nudge | arrows / `Shift`+arrows (10 px) |
| | Set opacity | `0`–`9` (`1`=10%, `0`=100%) |
| Arrange | Bring forward / backward | `Cmd+]` / `Cmd+[` |
| | Bring to front / send to back | `Cmd+Alt+]` / `Cmd+Alt+[` |
| | Group / Ungroup | `Cmd+G` / `Cmd+Shift+G` |
| | Frame selection | `Cmd+Alt+G` |
| | Show/hide | `Cmd+Shift+H` |
| | Lock/unlock | `Cmd+Shift+L` |
| Align | Left / H-center / Right | `Alt+A` / `Alt+H` / `Alt+D` |
| | Top / V-center / Bottom | `Alt+W` / `Alt+V` / `Alt+S` |
| | Distribute H / V spacing | `Alt+Shift+H` / `Alt+Shift+V` |
| | Tidy up | `Ctrl+Alt+T` |
| Auto layout | Add auto layout | `Shift+A` |
| | Remove auto layout | `Alt+Shift+A` |
| | Ignore auto layout (drag) | hold `Ctrl` while dragging in |
| Components | Create component | `Cmd+Alt+K` |
| | Detach instance | `Cmd+Alt+B` |
| | Show assets panel | `Alt+I` (`Alt+2`) |
| Boolean | Union / Subtract / Intersect / Exclude | `Alt+Shift+U` / `S` / `I` / `E` |
| Text | Bold / Italic / Underline / Strike | `Cmd+B` / `Cmd+I` / `Cmd+U` / `Cmd+Shift+X` |
| | Align L / C / R / Justify | `Cmd+Alt+L` / `T` / `R` / `J` |
| | Font size ±1 (±10 w/ Alt) | `Cmd+Shift+>` / `<` |
| | Create link | `Cmd+Shift+U` (was `Cmd+K`) |
| View | Zoom in / out | `Cmd+=` (or `+`) / `Cmd+-` (or `-`) |
| | Zoom to 100% | `Shift+0` |
| | Zoom to fit all | `Shift+1` |
| | Zoom to selection | `Shift+2` |
| | Previous / next frame | `Shift+N` / `N` (also Home/End/PgUp/PgDn) |
| | Zoom-to-pointer | pinch, `Cmd`+scroll, or `Z`+click (Alt+click zooms out) |
| | Pixel grid / layout grids | `Cmd+'` / `Ctrl+G` |
| | Rulers | `Shift+R` |
| | Outlines (wireframe) | `Cmd+Y` |
| | Show/hide UI | `Cmd+\` |
| | Layers / Assets panel | `Alt+1` / `Alt+2` |
| | Multiplayer cursors | `Cmd+Alt+\` |
| | Pick color (eyedropper) | `I` |
| Palette | Actions menu / Quick actions | `Cmd+K` / `Cmd+/` |
| | Shortcut panel | `Ctrl+Shift+?` |

### Paper shortcuts with direct evidence
* App shell: New File `Cmd+N`, tabs `Cmd+T/W/Shift+T`, Undo/Redo `Cmd+Z/Shift+Z`, Dashboard `Cmd+Shift+D`, zoom `Cmd +` / `Cmd -` / `Cmd 0`.
* Docs (https://paper.design/docs/svg, /docs/paste): Pen `P`, Move `M`, Enter = path edit, Esc = exit path edit, Create SVG (AI) `Cmd+Shift+J`, pixel snapping toggle `Cmd+Shift+'`, Paste `Cmd+V`, **Paste to replace `Cmd+R`** (Figma uses `Cmd+R` for rename. Sketch follows Figma: `Cmd+R` = rename, `Cmd+Shift+R` = paste to replace).

---

## 13. Export

Sources: https://help.figma.com/hc/en-us/articles/360040028114 (Export from Figma Design), https://help.figma.com/hc/en-us/articles/360040318013 (Export formats and settings), API `ExportSettings` (`format` PNG|JPG|SVG|PDF, `constraint {type: SCALE|WIDTH|HEIGHT, value}`, `suffix`, `contentsOnly`, `useAbsoluteBounds`, `svgOutlineText`, `svgIdAttribute`, `svgSimplifyStroke`), REST `GET /v1/images/:key?format=&scale=` (scale 0.01–4).

* **Export settings live on the layer** (any layer: frame, group, shape, text), as a list of `{scale|width|height, suffix, format}`. "+" adds `1x PNG`. Scale presets: 0.5x, 0.75x, 1x, 1.5x, 2x, 3x, 4x, or fixed `512w`/`512h`. Suffix defaults to empty for 1x and `@2x`/`@3x` style when typed. Filename = `<layer name><suffix>.<ext>`. Slashes in names create folders in zip exports.
* Formats: **PNG** (RGBA; transparent where no fill), **JPG** (quality option, no alpha: transparent becomes white), **SVG** (options: outline text vs `<text>`, include `id` attribute, simplify stroke), **PDF** (vector; text kept as text where possible). WEBP is not native.
* **What is rendered:** the layer and its descendants, cropped to the layer's bounds. A frame with clip content crops to the frame. A frame without clip, or a group/shape, includes **effects and strokes bleeding outside** (render bounds) unless "Use absolute bounds" is set. **Contents only** (default true) excludes overlapping layers that are not descendants. A frame's own fill is included, and the canvas background is never included.
* Selection with no export settings: the panel shows "Export <name>". `Cmd+Shift+E` opens "Export" for every layer with export settings on the page, as a zip/multi-file.
* **Copy as:** `Cmd+Shift+C` copy as PNG (scale: 1x or device-pixel-ratio, verify), "Copy as SVG", "Copy as code (CSS)", "Copy link". Copy as SVG uses the same SVG exporter, and the text is placed on the clipboard as `image/svg+xml` + plain text.
* **PDF multi-frame:** "Export frames to PDF" (File menu) exports **all top-level frames of the current page as one multi-page PDF**, one frame per page, in layers-panel order (verify; Sketch should sort top-to-bottom, then left-to-right, which is deterministic). Page size = frame size at 72 dpi (1 px = 1 pt).

**Sketch note:** add `export_settings: Vec<ExportSetting>` to `Node`. PNG/SVG already exist (`gpu.rs` readback, SVG writer). For PDF use a pure-Rust writer (`pdf-writer` or `printpdf`, MIT/Apache) emitting vector paths, one page per top-level frame sorted top-to-bottom then left-to-right. Copy as PNG/SVG goes via `arboard`.

---

## 14. Paper-specific behaviors worth matching

Sources: https://paper.design/compare/figma, https://paper.design/docs, https://paper.design/docs/svg, https://paper.design/docs/paste, https://paper.design/docs/tokens.

* **The canvas is real HTML/CSS** ("web-native, no translation step"). Every node is an element with CSS styles. Top-level containers are **artboards** (`create_artboard` MCP tool). Inner content is created via `write_html` and styled via `update_styles`. Consequences Sketch can mirror in naming and semantics:
  * **Box model:** `width/height` + `padding` + `border` follows CSS `box-sizing` (Paper presumably uses `border-box`, so the stroke/border sits *inside* the size, which equals Figma INSIDE stroke align). **Outlines** are a separate property (drawn outside, not affecting layout). Shadows are CSS `box-shadow` (multiple, `inset`), and filters are CSS `filter`/`backdrop-filter`.
  * **Layout is real CSS flexbox** (compare page). Paper's UI names follow CSS: direction row/column, `gap`, `justify-content` (start/center/end/space-between), `align-items` (start/center/end/stretch/baseline), `flex-wrap`, child `flex-grow`/fill vs fit-content (hug) vs fixed px, `position: absolute`. Figma's "auto layout" maps as: Hug = `fit-content`/auto size, Fill = `flex: 1` (primary) or `align-self: stretch` (counter), Ignore auto layout = `position: absolute`.
  * Text is HTML text: line-height and letter-spacing in CSS units, wrapping by the browser's line breaker, and `width: auto` for auto-width.
  * Overflow clipping = `overflow: hidden` on the container.
* **Color:** per-element color spaces in one file (sRGB and **Display P3**, `oklch()`, `display-p3`, hex). The picker uses **OkLCH/Oklab** (perceptually uniform) instead of HSB. Figma has a single file-wide sRGB/P3 toggle.
* **Shader effects:** a collection of GPU shader fills/effects (`shaders.paper.design`, an open-source package). Sketch could offer its own wgpu shader fills. Do not copy Paper's shader code.
* **Tokens:** design tokens (CSS-variable-like) style designs (`/docs/tokens`).
* **Paste:** images, SVG, text, Figma copy-paste and **HTML pasted as editable layers**. `Cmd+R` = paste to replace.
* **SVG/vector:** pixel snapping to half-pixels (`Cmd+Shift+'`). Pen inside an SVG adds a path, and outside creates a new SVG. Booleans, shape primitives and branching vector networks are roadmap only, so Paper is currently *less* capable than Figma here.
* **Agent/MCP-first:** tools such as `get_selection`, `get_screenshot` and `write_html`, and the AI "Create SVG" (`Cmd+Shift+J`).
* **Cloud-first:** files at `paper.design/file/<id>/<pageId>`, a Dashboard tab (`Cmd+Shift+D`), and a multi-tab desktop app.

**Sketch note:** use CSS vocabulary in the properties panel ("Flex row/column", gap, justify/align, padding, "Fit"/"Fill"/fixed). Treat stroke as border-box inside by default for frames, and offer an `outline` (outside, non-layout) as Paper does. Add an OkLCH color picker. SVG/HTML export can map 1:1 because the model is CSS-shaped.

---

## 15. File / project management (Figma)

Sources: https://help.figma.com/hc/en-us/articles/1500005554982 (File browser), https://help.figma.com/hc/en-us/articles/360041064174 (Drafts), https://help.figma.com/hc/en-us/articles/360041068174 (Delete and restore files).

* **File browser home ("Recents")** shows a grid of file cards: a 16:10-ish **thumbnail** (the file's cover = the first top-level frame of the first page by default, or a user-chosen "Set as thumbnail" frame), file name, and "Edited N minutes ago". Sorting: last viewed (default), last modified, alphabetical. Grid/list toggle. Filters by file type. Recents are per user, ordered by last *opened*.
* **Drafts:** a personal space where new files are created by default (`Cmd+N` from the browser or "+ Design file"). They can be moved to team projects. A file name defaults to "Untitled"; rename by double-clicking the name in the toolbar or via the card context menu.
* **Card context menu:** Open, Open in new tab, Copy link, Share, **Rename**, **Duplicate** ("<name> (Copy)", placed in the same location, stays in the browser without opening), Move to project, **Delete** (moves to trash), Remove from recent (only hides it from recents).
* **Trash ("Deleted"):** deleted files are listed in the Deleted view with the deletion date. **Restore** returns them to their original location. **Delete forever** is permanent. Figma does not auto-purge on a short timer for paid teams (verify the retention period). Multi-select works with Shift/Cmd-click on cards.
* **Version history:** autosave with named versions (`Cmd+Alt+S` "Save to version history"), restorable.
* Paper analog: the Dashboard tab lists cloud files.

**Sketch note:** local-first equivalent: a recents list in the app config dir (`recents.json`: path, last_opened, thumbnail path). Generate thumbnails on save by rendering the first top-level frame (or the whole page) to a 320×200 PNG via the existing offscreen path. "Delete" moves the file to an app-managed `.trash/` with metadata so it can be restored, or uses the OS trash via the `trash` crate.

---

## Appendix: implementation priorities for Sketch (suggested)
1. Selection model (§1) + group bounds + frame adopt: highest UX impact and cheap.
2. Constraints (§4) on frame resize, with snapshot-based recomputation.
3. Auto layout (§3) as a flex engine. This also unlocks Paper-style CSS vocabulary (§14).
4. Smart guides: equal spacing + Alt measurement (§9). Align/distribute (§8).
5. Layers panel DnD rules (§10), pages (§11), command palette + keymap table (§12).
6. Fills/strokes stack and gradients (§6), text auto-resize + ranges (§7).
7. Pen + booleans (§5), components (§2), PDF export (§13), recents/trash (§15).
