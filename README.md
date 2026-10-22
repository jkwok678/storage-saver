# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Dependency policy

`pnpm-workspace.yaml` sets `minimumReleaseAge` so pnpm only installs versions published at least
5 days ago. If a fresh release is genuinely needed, add it to `minimumReleaseAgeExclude`.

## Code quality

Run `pnpm check` before pushing; CI (`.github/workflows/ci.yml`) runs the same checks.

| Command                       | What it runs                                                  |
| ----------------------------- | ------------------------------------------------------------- |
| `pnpm typecheck`              | `tsc --noEmit`                                                |
| `pnpm lint` / `pnpm lint:fix` | Biome lint + format + import sorting                          |
| `pnpm fallow`                 | Fallow unused code/deps and duplication                       |
| `pnpm check:rust`             | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` |
| `pnpm check`                  | All of the above                                              |
