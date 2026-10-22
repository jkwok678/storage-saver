# Disk Map (sunburst) — Design

**Feature slug:** `disk-sunburst`  
**Requirements:** `docs/requirements/disk-sunburst.md`  
**Status:** Ready to implement

Locked open questions: tab label **Disk Map**; **4 rings** (depth 0–3); unique **hue per top-level folder**, tinted children; **right-click → Reveal in Finder**; files-only folder → **one unlabelled aggregate arc**; scans **lazy, one ring at a time** via `scan_directory`; centre label **always visible**.

---

## Approach summary

Add a `diskmap` view that paints a 4-ring SVG sunburst from the existing one-level `scan_directory` stream. A new `useSunburstScan` hook scans the chart root first (same Channel events as Browse), then **serially** lists each visible folder child with `refresh: false` so nested rings fill from the backend `STORED` tree without a new Rust command. Pure functions turn the listing map into annular SVG paths: Other grouping at 0.5% of parent, a Free-space arc only at `/`, files rolled into one unlabelled aggregate, pending slices hatched until bytes arrive. Geometry is first-principles polar math (no D3). Click / Enter / breadcrumb all write the shared `browsePath` in `App.tsx`.

---

## Alternatives considered

- **d3-shape / d3-hierarchy** — rejected. The chart is one SVG widget; polar arcs are ~80 lines, and `package.json` has no chart library.
- **New Tauri command returning N levels** — rejected by NFR-6. Nested data is already inside `STORED` after the root walk; extra IPC listings are enough.
- **Concurrent `scan_directory` for each child** — rejected. `SCAN_GENERATION` is global (“latest scan wins”); a second invoke cancels the first walk. Child listings must be queued.
- **Keep Browse mounted and reuse its `entries`** — rejected. Browse is one level, unmounts on tab switch, and resetting `useDirectoryScan` to `[]` would flash outer rings. Disk Map owns a path→listing map.

---

## Constraints from the current stack

`scan_directory` lists one folder, fills sizes from `STORED` when present, and walks only missing folders. `measure_tree` records **every nested folder** under those children, then `store_scan` grafts them. So after the chart-root scan **completes**, a child `scan_directory(path, refresh: false)` is `readdir` + cache lookup, not a second walk.

`SCAN_GENERATION` in `src-tauri/src/scan/mod.rs` is process-global. Starting a child scan while the root is still walking would abort that walk. **Never enqueue child listings until the parent job’s `Done` (or invoke settle).** Serialize every invoke.

`useDirectoryScan` today always `setEntries([])` when `request` changes. Disk Map must not reuse that hook for the tree; it would blank the chart on every nested path. Extract the Channel/invoke helper and give Disk Map its own map-based hook.

Ring geometry is **fixed for 4 bands from the first paint**. Outer rings appear into reserved space; inner radii never jump (EC-4).

---

## Components

### New — `src/diskmap/`

| File | Responsibility |
|---|---|
| `geometry.ts` | Polar ↔ cartesian, annular SVG `d`, full-circle split (SVG 360° arcs degenerate). |
| `color.ts` | Deterministic hue from folder name; child tint; CSS-facing HSL. |
| `layout.ts` | Listings → `LayoutNode[]`: Other group, files aggregate, free space, pending weights, depth cap. |
| `scanDirectory.ts` | Shared non-hook: start/cancel one `scan_directory` Channel. **Move** the invoke/event wiring out of `useDirectoryScan` into here. |
| `useSunburstScan.ts` | Serial BFS queue, path→listing map, rAF size batching, generation guard. |
| `SunburstChart.tsx` | SVG rings, centre label, tooltip, keyboard/pointer/contextmenu. |
| `DiskMapView.tsx` | Chrome: breadcrumbs, Refresh, status, error/empty/browser-dev states. Wires path + trash. |
| `diskmap.css` | Panel, tooltip, arcs, pending hatch, light/dark. |

### Modified

| File | Change |
|---|---|
| `src/browse/useDirectoryScan.ts` | Call `startDirectoryScan`; keep the same public hook API. |
| `src/app/App.tsx` | `View` += `"diskmap"`; `VIEWS` += `{ key: "diskmap", label: "Disk Map" }`; render `DiskMapView` with the same props as `BrowseView`. |
| `src/app/App.test.tsx` | Mock Disk Map; assert the tab and path plumbing. |

No Rust changes. No `BrowseView` / `LargestFilesView` / `AiAdvisorPanel` changes. `currentPath={browsePath}` already satisfies FR-13.

---

## Interfaces

### Shared scan helper

```ts
// src/diskmap/scanDirectory.ts  (imported by browse + diskmap)
export type ScanRequest = { path: string; refresh: boolean };
export type SizeUpdate = { path: string; bytes: number; error: string | null };

export type ScanHandlers = {
  isCurrent: () => boolean;
  onStarted: (entries: Entry[]) => void;
  onSized: (update: SizeUpdate) => void;
  onDone: () => void;
  onError: (message: string) => void;
};

/** Returns a cancel function that only flips isCurrent; backend cancel is the next generation or stop_scan. */
export function startDirectoryScan(request: ScanRequest, handlers: ScanHandlers): () => void;
```

`useDirectoryScan` keeps returning `{ entries, scanning, scanError }`. Move `ScanRequest` here and re-export from `useDirectoryScan.ts` so Browse imports stay stable.

### Hook

```ts
export type SunburstScan = {
  listings: ReadonlyMap<string, Entry[]>;
  rootEntries: Entry[];
  scanning: boolean;      // root or any nested job
  rootScanning: boolean;
  scanError: string | null; // root only; nested failures stay off the chart
  refresh: () => void;    // { path: root, refresh: true }
};

export function useSunburstScan(request: ScanRequest): SunburstScan;
```

`request` is stored in view state (same identity rule as Browse). Nested jobs always use `refresh: false`.

Queue item: `{ path: string; depth: number; refresh: boolean }`. Depth 0 is the chart root (produces ring 0). A job at depth `d` is listed so its children can occupy ring `d`. Do not enqueue depth `>= MAX_RINGS` (4).

After a job **Done**:

1. Group that listing (same rules as layout).
2. Enqueue each `kind === "folder"` child with `depth + 1` only if `depth + 1 < MAX_RINGS` **and** the child’s sweep around the **full circle** is `>= MIN_EXPAND_TURN` (2° / 360). Skip Other, files, free, errors, and `bytes === null`.

### Layout

```ts
export const MAX_RINGS = 4;
export const OTHER_THRESHOLD = 0.005;
export const MIN_EXPAND_TURN = 2 / 360;
export const VIEWBOX = 100;
export const CENTER = 50;
export const OUTER_RADIUS = 48;
export const HOLE_RATIO = 0.28; // hole radius / OUTER_RADIUS

export type ArcKind = "folder" | "files" | "other" | "free";

export type LayoutNode = {
  id: string;            // path, or `${parentPath}::__files__` / `__other__` / `__free__`
  name: string;          // folder name, "Files", "Other", "Free space"
  path: string | null;   // real path only for folders
  depth: number;         // 0..3
  startAngle: number;    // radians, 0 = 12 o'clock, clockwise
  endAngle: number;
  innerRadius: number;
  outerRadius: number;
  bytes: number | null;
  parentBytes: number;
  kind: ArcKind;
  hue: number | null;    // null for other/free/files
  pending: boolean;
  clickable: boolean;    // folder && !error
};

export type LayoutInput = {
  rootPath: string;
  listings: ReadonlyMap<string, Entry[]>;
  available: number | null; // free-space bytes; ignored unless rootPath === "/"
};

export function layoutSunburst(input: LayoutInput): LayoutNode[];
export function rootTotalBytes(rootPath: string, entries: Entry[]): number;
export function percentOfParent(bytes: number, parentBytes: number): string; // one decimal, e.g. "35.0%"
export function segmentAriaLabel(node: LayoutNode): string;
```

### Geometry

```ts
export function polar(cx: number, cy: number, r: number, a: number): { x: number; y: number };
/** Annular sector. `a0 === a1` → empty string. Sweep ≥ 2π − ε → two 180° rings. */
export function annularPath(cx: number, cy: number, r0: number, r1: number, a0: number, a1: number): string;
```

Angles: `0` at 12 o’clock, clockwise (`a - π/2` when converting to cartesian) so the first child starts at the top.

Radii (fixed 4 bands):

```
hole = OUTER_RADIUS * HOLE_RATIO
band = (OUTER_RADIUS - hole) / MAX_RINGS
inner(depth) = hole + depth * band + 0.4
outer(depth) = hole + (depth + 1) * band - 0.4
```

### Colour

```ts
export function hueFromName(name: string): number; // 0..359, 31-hash of UTF-16 code units
export function arcHsl(hue: number, depth: number): string;
// depth 0: hsl(h, 62%, 54%)
// depth d: sat = 62 - 6d, light = 54 + 7d  (clamped)
// dark theme overrides sat/light in CSS, not in JS
```

Ring-0 folders get `hueFromName(name)`. Descendants copy the **ring-0 ancestor** hue (passed down in layout), not a fresh hash. Other / free / files: no hue; CSS classes `.arc-other`, `.arc-free`, `.arc-files`.

### Chart + view props

```ts
type SunburstChartProps = {
  rootPath: string;
  nodes: LayoutNode[];
  totalBytes: number;
  scanning: boolean;
  onZoom: (path: string) => void;
  onReveal: (path: string) => void;
};

type DiskMapViewProps = {
  initialPath: string;
  onPathChange: (path: string) => void;
  available: number | null;
  trashed?: TrashedItem | null;
};
```

---

## Data model

No schema, no Rust types, no migrations.

Frontend-only:

```
listings: Map<rootOrChildPath, Entry[]>
```

`Entry` is unchanged (`src/browse/entries.ts`). Files never become layout folders. A files aggregate is synthetic (`kind: "files"`, `path: null`). Other is synthetic. Free space is synthetic from `available`, not from the scan.

**Backward compatibility:** Browse keeps calling `useDirectoryScan` with the same `ScanRequest`. Extracting `startDirectoryScan` is an internal move.

**Trash:** copy Browse’s `trashed` effect. If the trashed path is inside the chart root, re-issue `{ path: root, refresh: false }` (STORED already dropped it). If the chart root sits inside the trashed folder, zoom to `parentPath(trashed.path)` and notify `onPathChange`.

---

## Geometry, colour, Other, free space

### Grouping one listing (`groupListing`)

Inputs: `entries`, `parentSweep` (radians of the parent arc; `2π` at the chart root), `includeFree` (true only for chart root `/` with `available !== null`).

1. Split dirs vs files. Sum file `bytes ?? 0` → files aggregate if `> 0` or if there are files and **no** dirs (EC-3 / OQ-5: still emit one unlabelled arc even when all file bytes are 0, so the ring is not empty).
2. **Weight** of a dir: `bytes ?? 0`. Pending dirs (`bytes === null` and no error) stay **out of Other** and get a layout floor of `1` so they render as a sliver (FR-10). Errors with `bytes === 0` stay as their own non-clickable folder arc (no zoom), not Other.
3. `knownTotal = sum(weights of dirs + files aggregate) + (includeFree ? available : 0)`.
4. Other: every **non-pending** dir whose `weight / knownTotal < 0.005` (when `knownTotal > 0`) merges into one gray arc. Files aggregate is **not** merged into Other (OQ-5 wants it visible on files-only folders; mixed folders keep a distinct unlabelled slice).
5. Sort remaining folder arcs by weight descending, then name. Append files aggregate, Other, then Free space (free is last so it sits as a stable trailing wedge).
6. If `knownTotal === 0` and there is at least one child (EC-6): equal sweeps, all pending-styled. If there are zero children: **no nodes** (empty message is the view’s job, EC-2).

### Nesting

`layoutSunburst` walks the listing map recursively. A child’s angular span is `parentSweep * (childWeight / parentTotal)`. Missing listings produce no outer ring (lazy). Depth stops at 3. Bytes of deeper folders already sit in the ancestor’s `Entry.bytes`.

### Free space (FR-7, EC-7)

Only when `rootPath === "/"` and `available !== null`. Omit when `available` is null (do not draw 0 B). Not clickable. Label `"Free space"`. Sweep uses `available / knownTotal`.

### Centre (OQ-7)

Always-on HTML overlay in the hole, not SVG text:

- Name: `breadcrumbs(rootPath).at(-1).label` (“This Mac” at `/`).
- Size: `formatBytes(sum of root listing weights excluding free)`. While `rootScanning` and every child is still `bytes === null`, show “Measuring…”.
- Empty listing, not scanning, no error: replace the chart with “This folder is empty.” (same copy as Browse).

### Progressive paint (FR-9, FR-10, NFR-7, EC-4)

- First `Started` on the root writes `listings.set(root, entries)` and React paints ring 0. Do **not** wait for `Done`.
- `Sized` events go through an rAF map (same idea as `batchSizeUpdates`) and patch that listing in place. Node `id`s are stable, so SVG `<path>`s update `d` / class without remounting the `<svg>`.
- Nested listings are **added** to the map, never replacing the root array with `[]`.
- Path change or Refresh: bump generation, drop nested keys, keep showing the previous root ring until the new `Started` (avoids a blank flash). Then replace the root listing.
- No CSS transition on `d`. Hover/focus uses CSS `filter` / stroke (NFR-1). Optional `opacity` 120ms on the svg when `rootPath` changes only.

---

## Interactions

| Input | Behaviour |
|---|---|
| Hover | Tooltip: name, `formatBytes`, `percentOfParent` (AC-4). Pending: “Measuring…”. Other: “Other” + combined size. |
| Click / Enter / Space | `kind === "folder" && clickable` → `onPathChange(path)` and new scan request `{ path, refresh: false }`. |
| Breadcrumb | Same `breadcrumbs()` segments as Browse (“This Mac”, then parts). Current crumb is text; ancestors are buttons. |
| Refresh | `{ path: root, refresh: true }`. Disabled while `scanning`. Nested follow-up jobs stay `refresh: false`. |
| Right-click | `preventDefault` + `revealItemInDir(path)` for folder paths (OQ-4). Failures go to the status line (same as Largest Files). Ignore Other / files / free. |
| Tab | Each clickable path has `tabIndex={0}` and `aria-label` from `segmentAriaLabel`. |

### Accessibility (FR-14, FR-15, NFR-5)

`role="img"` on the SVG would hide interactive descendants. Wrap the chart in `<figure aria-label={summary}>` where `summary` is `"Disk map of {name}, {size}"`. The SVG has no `role="img"`. That meets the intent of NFR-5 without breaking FR-14.

---

## Error handling & edge cases

| ID | Handling |
|---|---|
| EC-1 | Root `scanError` replaces the chart; `className="error"` (existing). Nested listing failures: skip that outer ring, keep the parent arc. |
| EC-2 | Zero children, not scanning, no error → “This folder is empty.” No arcs. |
| EC-3 / OQ-5 | No dirs, only files → one unlabelled `kind: "files"` arc (tooltip/aria name `"Files"`). Not clickable. |
| EC-4 | rAF batching; stable ids; do not `setEntries([])` on nested jobs. |
| EC-5 | `annularPath` splits sweeps `≥ 2π − 1e-6` into two 180° sectors. Clip `overflow: hidden` on the panel, not the svg. |
| EC-6 | All weights 0 → equal placeholder ring, pending style. No division by zero (`parentTotal` fallback `1`). |
| EC-7 | `available === null` → no free arc. |
| EC-8 | `runningInApp() === false` → `"Open Storage Saver to read this disk."` (helper already does this). |
| EC-9 | `MAX_RINGS = 4`; deeper bytes stay in the ancestor. |
| EC-10 | `scanGen` ref on the hook; queue replaced on path/refresh; stale Channel messages ignored; next `scan_directory` bumps `SCAN_GENERATION`. |
| Rapid nested enqueue | One in-flight invoke. Queue is a FIFO array. Path change clears the queue except the new root job. |

---

## Implementation plan

Each step is one commit. Tests land in the same commit as the code.

1. **Extract `startDirectoryScan`** — `src/diskmap/scanDirectory.ts`, rewire `src/browse/useDirectoryScan.ts`, add `scanDirectory.test.ts` (mock `@tauri-apps/api/core` Channel). Browse tests still mock the hook. *(enables NFR-6 reuse)*

2. **Geometry** — `geometry.ts` + `geometry.test.ts`: polar, empty sweep, 90° sector path commands, full-circle split. *(FR-2)*

3. **Colour + percent/aria helpers** — `color.ts` + `layout.ts` helpers + tests: same name → same hue; child tint; `percentOfParent(4.2e9, 12e9) === "35.0%"`; aria copy. *(FR-3, FR-4, FR-15)*

4. **Layout** — `layoutSunburst` tests covering Other (AC-6), free space omit/include (FR-7, EC-7), files-only (EC-3), mixed files+dirs, pending floor (FR-10), empty → `[]` (EC-2), all-zero (EC-6), depth cap 4 (EC-9), 99% segment (EC-5), 10k tiny children grouped so output length stays small (NFR-2). *(FR-2, FR-8, OQ-2, OQ-3, OQ-5)*

5. **`useSunburstScan`** — serial queue, generation, rAF patch, no clearing of sibling listings, child jobs only after parent Done, expand cap `MIN_EXPAND_TURN`, `refresh()` uses `refresh: true` only on root. Tests with a fake `startDirectoryScan` that records call order. *(FR-9, FR-11, NFR-3, NFR-6, NFR-7, EC-4, EC-10, OQ-6)*

6. **`DiskMapView` chrome** — breadcrumbs, Refresh, status (`scanStatus` on root entries; while nested jobs run, `"Reading nested folders…"`), empty/error/dev-mode, trash effect. Mock the hook. Tests for empty, error, refresh payload, breadcrumb `onPathChange`, trash parent jump. *(FR-6, FR-11, EC-1, EC-2, EC-8)*

7. **`SunburstChart`** — SVG paths from nodes, centre label, CSS hover, tooltip, figure `aria-label`, focusable folders, Enter/Space zoom, contextmenu reveal. Tests with a fixture `LayoutNode[]`. *(FR-2, FR-4, FR-5, FR-14, FR-15, OQ-4, OQ-7, NFR-4, NFR-5)*

8. **Wire `App.tsx`** — `diskmap` view, label “Disk Map”, same props as Browse, App test for the tab. Path stays in `browsePath` (FR-12, FR-13). *(FR-1, FR-12, FR-13, OQ-1)*

9. **`diskmap.css`** — match `.browse` location row + `.largest-panel` chrome; `--arc-other`, `--arc-free`, pending hatch; `prefers-color-scheme` dark (AC-12, NFR-4). No new test beyond chart class names already asserted.

---

## Testing strategy

Match existing style: Vitest + Testing Library, files next to source, mock Tauri/hooks at the view boundary, keep layout/geometry as plain function tests (like `entries.test.ts` / `format.test.ts`).

| Layer | File | What |
|---|---|---|
| Unit | `geometry.test.ts` | Path math, full-circle, zero sweep. |
| Unit | `color.test.ts` | Deterministic hue; tint monotonic in depth. |
| Unit | `layout.test.ts` | All grouping/edge cases above; 10k children < 50 ms (`performance.now()` assertion with a generous 50 ms budget). |
| Unit | `scanDirectory.test.ts` | Started/sized/done; ignore events after cancel; `errorText` on invoke reject. |
| Hook | `useSunburstScan.test.ts` | `renderHook`: root then serial children; stale gen dropped; refresh flag; listings survive size patches. |
| View | `DiskMapView.test.tsx` | Mock hook, like `BrowseView.test.tsx`. |
| View | `SunburstChart.test.tsx` | Tooltip, zoom, keyboard, reveal mock, empty centre. |
| App | `App.test.tsx` | Third nav button; switching to Disk Map renders the mock. |

No Rust tests. No e2e. Manual AC-12 (dark mode) and AC-1 on a real disk after the UI exists.

Coverage: new `src/diskmap/**` is inside `vitest.config.ts` `include`. View tests should click the interesting branches so `pnpm test:coverage` / fallow stay green.

---

## Risks

| Risk | Mitigation |
|---|---|
| Nested `scan_directory` cancels the root walk | Enqueue children only after parent Done; one in-flight invoke. |
| Thousands of child listings | Expand only folders with `≥ 2°` of the **full** circle; Other already caps per-parent clutter. |
| Inner rings jump when outer rings load | Fixed 4-band radii from paint one. |
| `role="img"` vs keyboard | `aria-label` on `<figure>`, interactive `<path>`s stay in the tree. |
| Refresh looks empty | Keep last root ring until new `Started`; drop only nested keys. |
| Hash hues collide | Acceptable; FR-3 requires stability, not uniqueness. |
| 10k-child layout budget | Other grouping is O(n) with no SVG until after group; test the 50 ms bound. |

---

## FR coverage

| FR | Step |
|---|---|
| FR-1 Disk Map tab | 8 |
| FR-2 sunburst rings | 2, 4, 7 |
| FR-3 deterministic colour | 3, 4 |
| FR-4 tooltip | 3, 7 |
| FR-5 click to zoom / `browsePath` | 7, 8 |
| FR-6 breadcrumbs | 6 |
| FR-7 free space at `/` | 4, 7 |
| FR-8 Other grouping | 4 |
| FR-9 partial scan | 5, 7 |
| FR-10 pending appearance | 4, 7, 9 |
| FR-11 Refresh | 5, 6 |
| FR-12 shared path | 8 |
| FR-13 AI `currentPath` | 8 (existing prop) |
| FR-14 keyboard | 7 |
| FR-15 aria-label | 3, 7 |

NFR-1 CSS hover; NFR-2 layout test; NFR-3/6 serial `scan_directory`; NFR-4 step 9; NFR-5 figure label; NFR-7 first paint on `Started`.
