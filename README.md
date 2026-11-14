# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Platforms

Storage Saver runs on macOS and Windows 10/11 (x64).

| Platform | Build                                | Output                          |
| -------- | ------------------------------------ | ------------------------------- |
| macOS    | `pnpm tauri build --bundles app,dmg` | `.app` and `.dmg`               |
| Windows  | `pnpm tauri build --bundles nsis`    | per-user `-setup.exe` installer |

Building on Windows needs the Visual Studio Build Tools with the "Desktop development with C++"
workload and the Rust MSVC toolchain. The installer bootstraps WebView2 if it is missing.

## Dependency policy

`pnpm-workspace.yaml` sets `minimumReleaseAge` so pnpm only installs versions published at least
5 days ago. If a fresh release is genuinely needed, add it to `minimumReleaseAgeExclude`.

## Code quality

Run `pnpm check` before pushing; CI (`.github/workflows/ci.yml`) runs the same checks.

| Command                       | What it runs                                                  |
| ----------------------------- | ------------------------------------------------------------- |
| `pnpm typecheck`              | `tsc --noEmit`                                                |
| `pnpm lint` / `pnpm lint:fix` | Biome lint + format + import sorting                          |
| `pnpm test` / `pnpm test:coverage` | Vitest unit and component tests |
| `pnpm fallow` | Fallow unused code/deps, duplication, and complexity (run `test:coverage` first) |
| `pnpm check:rust`             | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` |
| `pnpm check`                  | All of the above                                              |
