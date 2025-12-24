The missing tools in CMake:
1. `cmk new`: Create a new CMake boilerplate project.
2. `cmk run`: Builds and runs a specified executable target, getting rid of the build directory and binary path.
3. `cmk build`: Automatically discovers the project's build directory and invokes the build process from any subdirectory.
4. `cmk build-tu`: Speeds up iteration by compiling a single source file (translation unit) on its own.

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
```
