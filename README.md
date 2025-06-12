The missing tools in CMake:
1. `cmk new`: Create a new CMake boilerplate project.
2. `cmk run`: Builds and runs a specified executable target, getting rid of the build directory and binary path.
3. `cmk build`: Automatically discovers the project's build directory and invokes the build process from any subdirectory.
4. `cmk build-tu`: Speeds up iteration by compiling a single source file (translation unit) on its own.

TODO:
1. Support discovering nested build subdirectories from the project root.
