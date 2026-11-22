# Contributing to Neutron

Thank you for contributing to Neutron!

## Conventional Commits & Versioning

Neutron follows the [Conventional Commits](https://www.conventionalcommits.org/) specification to automate **Semantic Versioning** and **changelog generation** (`CHANGELOG.md` and GitHub Releases).

### Commit Format

```text
<type>[optional scope][!]: <description>

[optional body]

[optional footer(s)]
```

### Common Types

| Type | Effect on Version | Description |
| :--- | :--- | :--- |
| `feat` | **Minor** (`x.Y.0`) | A new user-facing feature |
| `fix` | **Patch** (`x.y.Z`) | A bug fix |
| `perf` | **Patch** (`x.y.Z`) | A performance improvement |
| `refactor` | None | Code refactoring without behavior changes |
| `docs` | None | Documentation updates |
| `style` | None | Formatting or style changes |
| `test` | None | Adding or updating tests |
| `build` | None | Build system, dependencies, or packaging changes |
| `ci` | None | GitHub Actions / CI configuration |
| `chore` | None | Maintenance tasks |
| `revert` | Patch / Minor | Reverts a previous commit |

### Breaking Changes

Append `!` after the type/scope or include `BREAKING CHANGE:` in the commit footer to trigger a **Major** (`X.0.0`) release:

```text
feat!: rename NEUTRON_TIMEOUT to NEUTRON_LAUNCHER_TIMEOUT
```

### Examples

```text
feat(ui): add DLSS preset selector for supported NVIDIA GPUs
fix(launch): restore host LD_LIBRARY_PATH before exec'ing Proton
build(arch): update PKGBUILD dependencies for libadwaita
docs(readme): add installation steps for AppImage
```

## Development Workflow

1. Fork the repository and create a branch from `main`.
2. Ensure formatting, linting, and all unit tests pass before opening a pull request:

   ```bash
   make check
   ```

   Which runs:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test --locked
   ```

3. Use a Conventional Commit title for your pull request (and commit messages).

## Testing & UI Preview

To preview the pre-launch countdown window during development without launching a Steam game, pass `--test` (or `-t`) with an optional Steam App ID:

```bash
cargo run -- --test 3669870
```

If no App ID is given, Neutron falls back to the App ID from the current Steam environment or `3669870`.
