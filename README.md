The missing tools in CMake:
1. `cmk new`: Create a new CMake boilerplate project.
2. `cmk run`: Builds and runs a specified executable target, getting rid of the build directory and binary path.
3. `cmk build`: Automatically discovers the project's build directory and invokes the build process from any subdirectory.
4. `cmk build-tu`: Speeds up iteration by compiling a single source file (translation unit) on its own.
5. `cmk fmt`: Formats C/C++ source files with `clang-format`. Pass a positional source path to format a single file (`cmk fmt src/foo.cpp`); otherwise selects via `--all` (all tracked files), `--staged`, or `--unstaged`. Files matching `[fmt] ignore` glob patterns in `.cmk.toml` are skipped.
6. `cmk init`: Scaffold a `.cmk.toml` in the project root with commented-out examples for `[build]`, `[vars]`, `[env]`, `[fmt]`, `[lint]`. Pass `-f/--force` to overwrite.
7. `cmk completions <shell>`: Print shell completions to stdout. Supports `bash`, `zsh`, `fish`, `powershell`, `elvish`. Example: `cmk completions zsh > ~/.zfunc/_cmk`.
8. `cmk lint`: Lints C/C++ source files with `clang-tidy` against the build directory's `compile_commands.json`. Same `--all`/`--staged`/`--unstaged` selection as `fmt`, plus `--fix` (serial), `-W/--warnings-as-errors`, and `-b/--build` to pick the build dir. To target a single TU, pass a positional source path (`cmk lint src/foo.cpp`) or `-i/--interactive` to pick from `compile_commands.json` via fzf. Results are cached per-file under `<build>/.cmk-lint-cache/`; unchanged files (same source mtime/size, cdb mtime, `.clang-tidy` chain, and CLI args) replay the cached output without re-invoking clang-tidy. Pass `--no-cache` to bypass. Honours `[lint]` in `.cmk.toml` (`ignore`, `warnings_as_errors`, `header_filter`, `extra_args`).

Package management (CPM):
- `cmk add owner/repo`: Track a GitHub release in the global package index (`~/.config/cmk/pkg.json`).
- `cmk get name`: Print the cached release for a tracked package (alias or full `owner/repo`).
- `cmk update`: Refresh the latest release tags for all tracked packages and the bundled CPM bootstrap script. Pass `-p/--project` to also scan the root `CMakeLists.txt` for `CPMAddPackage("gh|gl|bb:owner/repo#tag")` URIs, query GitHub for each, print a diff, and (with confirmation, or `-y` to skip) splice in the new versions while preserving comments and formatting.

Requirement:
1. Only works with CMake projects with `Ninja` as the generator(`Ninja Multi-Config` is not supported).
2. `fzf` is required for interactive selection.
3. The project root discovery only works in a git repository.

Environment Variables:
- `CMK_DEFAULT_JOBS`: The default number of build jobs to use. If not set, it defaults to the number of available CPU cores minus one.

Example of `.cmk.toml`:

```toml
[vars]
DEPS_DIR = "${PROJECT_ROOT}/.deps"
DEPS_INSTALL = "${DEPS_DIR}/install"

[env]
PATH = { prepend = ["${DEPS_INSTALL}/bin"] }
CPATH = { prepend = ["${DEPS_INSTALL}/include"] }
PKG_CONFIG_PATH = { prepend = ["${DEPS_INSTALL}/lib/pkgconfig"] }
LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib"] }

[env.macos]
DYLD_LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib"] }

[env.linux]
LD_LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib", "${DEPS_INSTALL}/lib64"] }
PKG_CONFIG_PATH = { prepend = ["${DEPS_INSTALL}/lib64/pkgconfig"] }
LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib64"] }

[build]
default = "build/debug"  # used when PWD isn't inside a build dir and there are multiple

[fmt]
ignore = ["third_party/**", "*.pb.h"]

[lint]
ignore = ["third_party/**", "*.pb.h"]
warnings_as_errors = false
header_filter = "^(src|include)/"
extra_args = ["-quiet"]
```
