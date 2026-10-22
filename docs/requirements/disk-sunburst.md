# Disk Sunburst View — Requirements

**Feature slug:** `disk-sunburst`  
**Status:** Draft — awaiting implementation design  
**Date:** 2026-10-22

---

## Problem statement

Users cannot quickly see, at a glance, which folders and files consume the most space and how nested
directories contribute to the total. The existing Browse view presents a flat, sortable list of
immediate children of a single folder, requiring manual navigation to understand how space is
distributed across a subtree. The Largest Files view shows individual files but gives no structural
picture of where waste is concentrated. A radial disk-map (sunburst / concentric-ring chart) — like
DaisyDisk on macOS — lets a user immediately see the shape of their disk usage as nested arcs whose
arc length is proportional to bytes, without visiting every folder manually.

---

## In scope

- A new **Disk Map** tab/view in the app's top-level navigation (alongside Browse and Largest Files).
- A multi-ring radial chart where each ring represents one level of folder depth starting from the
  current root.
- Hover interaction: display name, size, and percentage of parent for the hovered segment.
- Click-to-zoom: clicking a folder segment makes that folder the chart's root, drilling deeper.
- A navigation breadcrumb or back button that lets the user zoom back out.
- The chart root is kept in sync with the Browse view's current path — switching between tabs
  preserves the path.
- A "free space" arc representing unallocated disk space when the chart root is the boot disk root
  (`/`).
- Light and dark mode support matching the existing app theme.

## Out of scope

- 3-D or animated transitions (flat 2-D arcs only; CSS transitions on zoom are allowed).
- File-level arcs (only folders are rings; files contribute to their parent folder's arc, consistent
  with the backend `Tree` model which does not store file nodes).
- Deleting or trashing items directly from the chart (handled exclusively by existing Browse /
  Largest views and the AI Advisor).
- Windows or Linux support — app targets macOS only.
- Displaying more than **5 depth levels** at once (to preserve readability at normal window sizes).
- A standalone data scan initiated exclusively for this view — the chart MUST reuse data already
  gathered by the directory-scan infrastructure (`STORED` tree in the backend).

---

## Functional requirements

**FR-1** The app MUST add a "Disk Map" entry to the top-level view navigation (the `VIEWS` array in
`App.tsx`), making it a first-class peer of Browse and Largest Files.

**FR-2** The Disk Map view MUST display a sunburst (radial concentric-ring) chart in which:
- The innermost ring (ring 0) represents the direct children of the current root path.
- Each successive ring represents one level of nesting within the segment that contains it.
- Arc length of each segment MUST be proportional to that item's total byte size relative to its
  parent ring's total.

**FR-3** Each segment's color MUST be assigned deterministically by folder name so that the same
folder always gets the same hue across navigations and reloads.

**FR-4** Hovering a segment MUST display a tooltip showing, at minimum:
- The folder name.
- Its total size in human-readable form (using the existing `formatBytes` utility from
  `src/shared/format.ts`).
- Its percentage of its parent segment's size, rounded to one decimal place (e.g. `14.3%`).

**FR-5** Clicking a folder segment MUST set that folder as the new chart root and update the shared
`browsePath` state in `App.tsx` (so that switching to Browse immediately shows that folder).

**FR-6** The view MUST display a breadcrumb above or beside the chart showing the path from the
initial root down to the current chart root, using the same path components as the existing
`Breadcrumbs` component in `BrowseView`. Clicking a breadcrumb segment MUST zoom back out to that
ancestor.

**FR-7** When the chart root is the filesystem root `/`, the chart MUST display a "Free space" arc
inside the innermost ring (or as a distinct center label) representing `DiskSummary.available`
bytes.

**FR-8** Segments whose bytes represent less than **0.5 %** of the parent ring's total MUST be
grouped into a single gray "Other" arc per ring to prevent visual clutter. This grouping is
purely visual — the "Other" arc is not clickable.

**FR-9** The chart MUST reflect a scan-in-progress state: while the backend is still measuring
folder sizes (i.e., entries have `bytes === null`), the view MUST show a progress indicator (spinner
or status text) and render whatever partial data is available. It MUST NOT wait for the scan to
finish before showing any arcs.

**FR-10** While the scan is in progress, segments whose sizes are not yet known MUST render with a
visually distinct "pending" appearance (e.g., a lighter, desaturated fill or hatch pattern) so the
user understands the data is incomplete.

**FR-11** The view MUST provide a **Refresh** button that triggers a fresh `scan_directory` call
(with `refresh: true`) for the current chart root, discarding cached sizes, matching the existing
Refresh button in BrowseView.

**FR-12** If the user switches from Browse to Disk Map (or vice versa), the current path MUST be
preserved — both views share `browsePath` from `App.tsx`.

**FR-13** The chart MUST expose its current root path to the AI Advisor via the existing
`currentPath` prop already passed to `AiAdvisorPanel` in `App.tsx`.

**FR-14** The chart MUST be keyboard-navigable: segments MUST be focusable and MUST respond to
Enter/Space to trigger the same zoom behavior as a click.

**FR-15** Each segment MUST have an accessible `aria-label` describing the folder name and size (e.g.
`"Documents, 14.3 GB, 22.1% of parent"`).

---

## Non-functional requirements

**NFR-1** The chart MUST render at least 30 fps during hover and zoom transitions on a 2021 M1
MacBook Pro running macOS 14 (Sonoma). SVG rendering with CSS transitions is acceptable; a canvas
fallback is not required.

**NFR-2** Building the chart data structure from an already-completed scan result (a flat list of
`Entry` objects from `useDirectoryScan`) MUST complete in under **50 ms** for a directory
containing up to 10,000 children at any single level.

**NFR-3** The view MUST NOT initiate a new Tauri IPC call to scan folders the backend `STORED` tree
already knows about. It MAY call `scan_directory` for a path that is not yet in cache (same rule
as BrowseView).

**NFR-4** The chart MUST correctly handle macOS light and dark mode by reading CSS custom properties
or `prefers-color-scheme`, matching the patterns in `App.css`.

**NFR-5** The chart SVG element MUST have `role="img"` and an `aria-label` summarising the chart
root and total size, so screen-reader users receive a meaningful description.

**NFR-6** The implementation MUST NOT introduce any new Rust Tauri commands — the sunburst MUST be
driven entirely from data already exposed by `scan_directory` (Channel-streamed `ScanEvent`). If
hierarchical data beyond one level is needed, the frontend MUST request child levels iteratively
using existing `scan_directory` calls.

**NFR-7** The initial render of the chart (first paint showing at least the innermost ring) MUST
occur within **200 ms** of the first `ScanEvent { event: "started" }` message being received on the
frontend.

---

## Edge cases and error states

**EC-1 Permission denied** — If `scan_directory` returns a `scanError`, the chart MUST display the
error string in place of the chart, styled with the `.error` CSS class already used across the app.

**EC-2 Empty folder** — If the scanned directory has zero children, the chart center MUST display
"This folder is empty." instead of arcs.

**EC-3 Single large file** — If the folder contains files but no subdirectories (all entries have
`isDir: false`), the innermost ring MUST still render correctly, representing the aggregate file
content of the folder as a single unlabelled arc (files have no children to drill into).

**EC-4 Scan still in progress** — Arcs that do not yet have a size (`bytes === null`) MUST be
rendered in a pending state (see FR-10). As sizes stream in, the chart MUST update in place without
a full re-render flash; animation-frame batching (like `batchSizeUpdates` in `useDirectoryScan.ts`)
MUST be applied.

**EC-5 One segment dominates** — A folder that is 99%+ of the parent MUST still render as a full
arc without layout overflow or clipping.

**EC-6 Total bytes = 0** — If all children have `bytes === 0` or `null`, no division-by-zero error
MUST occur; the chart MUST show a uniform placeholder ring or the empty-folder message.

**EC-7 Disk summary unavailable** — If `useDiskSummary` has not yet resolved (summary is `null`),
the "Free space" arc (FR-7) MUST be omitted rather than showing 0 B or NaN.

**EC-8 Running outside Tauri** — If `runningInApp()` returns `false` (e.g. browser dev mode), the
view MUST display "Open Storage Saver to read this disk." consistent with other views.

**EC-9 Very deep nesting** — Paths deeper than 5 levels below the current chart root MUST NOT be
rendered (depth is capped per the scope constraint); their bytes contribute to their outermost
visible ancestor.

**EC-10 Rapid path changes** — If the user clicks multiple segments faster than the backend can
respond, only the most recent `scan_directory` invocation's results MUST be displayed; stale
in-flight scans MUST be cancelled via the generation-counter pattern used in `useDirectoryScan`.

---

## Acceptance criteria

**AC-1 — Happy path: viewing a folder**  
*Given* the user has scanned `/Users/alice/Documents` in Browse,  
*When* they switch to Disk Map,  
*Then* a sunburst chart appears with `/Users/alice/Documents`'s immediate children as the innermost
ring, each arc proportional to its measured byte count.

**AC-2 — Drill down**  
*Given* the Disk Map is showing `/Users/alice/Documents`,  
*When* the user clicks a segment labelled "Projects",  
*Then* the chart re-roots at `/Users/alice/Documents/Projects`, the breadcrumb updates to show the
full path, and `browsePath` in `App.tsx` is set to `/Users/alice/Documents/Projects`.

**AC-3 — Breadcrumb navigation**  
*Given* the user has drilled down two levels from `/`,  
*When* they click the first breadcrumb segment (`/`),  
*Then* the chart re-roots at `/` and the breadcrumb collapses back to the root.

**AC-4 — Hover tooltip**  
*Given* a segment for "Movies" containing 4.2 GB of a 12 GB parent,  
*When* the user hovers it,  
*Then* a tooltip displays "Movies", "4.2 GB", and "35.0%".

**AC-5 — Free space arc**  
*Given* the chart is rooted at `/` and `DiskSummary.available` is 80 GB,  
*Then* a "Free space" arc representing 80 GB is visible, labelled "Free space", and not clickable.

**AC-6 — "Other" grouping**  
*Given* a folder has 50 children where 30 of them each represent less than 0.5% of the folder's
total size,  
*Then* those 30 are combined into a single grey "Other" arc per ring; hovering it shows "Other"
and the combined size.

**AC-7 — Scan-in-progress state**  
*Given* a slow directory scan is underway,  
*When* the view is active during the scan,  
*Then* arcs with known sizes render at their correct proportions while arcs still pending show a
visually distinct (desaturated/hatched) fill, and no error or blank screen appears.

**AC-8 — Path sync with Browse**  
*Given* the user is in Browse at `/Users/alice`,  
*When* they switch to Disk Map and then back to Browse,  
*Then* Browse is still showing `/Users/alice` (path is preserved in both directions).

**AC-9 — Keyboard zoom**  
*Given* a segment is focused via Tab key,  
*When* the user presses Enter,  
*Then* the chart re-roots at that folder, identical to a mouse click.

**AC-10 — Empty folder**  
*Given* the current path is a folder with no children,  
*When* the Disk Map view renders,  
*Then* the chart area shows the text "This folder is empty." instead of any arcs.

**AC-11 — Error state**  
*Given* `scan_directory` returns a `scanError` (e.g. permission denied on `/private`),  
*When* the Disk Map view is shown,  
*Then* the error message is displayed in place of the chart, styled consistently with other error
states in the app.

**AC-12 — Dark mode**  
*Given* macOS is set to dark appearance,  
*When* the Disk Map view is open,  
*Then* background, text, tooltip, and arc colours all adapt to the dark palette without hard-coded
light-only colours.

**AC-13 — Refresh**  
*Given* the user clicks Refresh,  
*Then* a new `scan_directory` call with `refresh: true` is made for the current chart root, and the
chart updates with fresh data when the scan completes.

---

## Open questions

All questions below carry a recommended default. A human MUST confirm or override each one before
the architect designs the implementation.

| # | Question | Recommended default |
|---|----------|---------------------|
| OQ-1 | **Tab label** — Should the new tab be labelled "Disk Map" or "Map"? | **"Disk Map"** — unambiguous; consistent with DaisyDisk's brand language. |
| OQ-2 | **Depth limit** — Scope says max 5 rings. Is 4 rings (0–3) more appropriate for typical window sizes? | **4 rings** — keeps arcs legible; users can zoom in for deeper levels. |
| OQ-3 | **Colour scheme** — Should each top-level folder get a unique hue with child rings being lighter tints, or should hue be assigned only by depth with brightness varying by position? | **Unique hue per top-level folder, tinted for children** — matches DaisyDisk and makes sibling groups visually distinct. |
| OQ-4 | **Reveal in Finder** — Should right-clicking a segment reveal it in Finder (using the existing `revealItemInDir` API)? | **Yes, via context menu or right-click** — consistent with how Largest Files handles this. |
| OQ-5 | **Files-only parents** — When drilling into a folder that contains only files (no subfolders), should the chart simply show a single arc for "Files" with the aggregate size, or show an empty inner ring with a message? | **Single unlabelled arc for aggregate file content** — minimises dead-ends; user can switch to Browse to see individual files. |
| OQ-6 | **Scan scope** — Should the Disk Map fetch child-level data one ring at a time (incremental) or all at once up to the depth limit? | **One ring at a time, lazily** — consistent with `useDirectoryScan` which already handles one level; avoids a new backend command. |
| OQ-7 | **Center label** — Should the chart centre display the root folder name and total size, or only show it on hover? | **Always visible in centre** — gives persistent context without requiring hover. |

---

## Integration with existing views

### Browse view (`src/browse/BrowseView.tsx`)
- Browse and Disk Map share `browsePath` from `App.tsx`. No changes to `BrowseView` are needed
  beyond confirming that `onPathChange` is threaded through both places.
- The Disk Map's zoom navigation calls the same `onBrowsePath` setter in `App.tsx`, so Browse
  always reflects the last path the user visited in either view.

### Largest Files view (`src/largest/LargestFilesView.tsx`)
- No direct integration. The two views answer different questions (structural vs. file-level).
- A future enhancement could highlight the largest files from `useLargestScan` as overlaid
  annotations on the sunburst, but this is **out of scope** for this feature.

### AI Advisor (`src/ai/AiAdvisorPanel.tsx`)
- `AiAdvisorPanel` already receives `currentPath` from `App.tsx`. When the Disk Map is the active
  view, `browsePath` should still be the authoritative current path, so no changes are needed.
- The advisor's snapshot (`storage_snapshot` in `snapshot.rs`) exposes `top_folders` which it
  already computes from `STORED`. The sunburst does not change this contract.

### App.tsx navigation
- Add `"diskmap"` (or `"map"`) to the `View` union type.
- Add `{ key: "diskmap", label: "Disk Map" }` to the `VIEWS` array.
- Add the `DiskMapView` component to `CurrentView`'s dispatch.
- Pass `browsePath`, `onBrowsePath`, `available`, and `trashed` to `DiskMapView` (same props as
  `BrowseView`).

### Tauri backend
- No new Rust commands are required per NFR-6. The sunburst drives entirely from `scan_directory`
  events already exposed.
