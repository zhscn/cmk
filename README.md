The missing tools in CMake:
1. `cmk new`: Create a new CMake boilerplate project.
2. `cmk run`: Builds and runs a specified executable target, getting rid of the build directory and binary path.
3. `cmk build`: Automatically discovers the project's build directory and invokes the build process from any subdirectory.
4. `cmk build-tu`: Speeds up iteration by compiling a single source file (translation unit) on its own.

Requirement:
1. Only works with CMake projects with `Ninja` as the generator(`Ninja Multi-Config` is not supported).
2. `fzf` is required for interactive selection.
3. The project root discovery only works in a git repository.
