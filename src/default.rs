pub const GIT_IGNORE: &str = r#"# Prerequisites
*.d

# Compiled Object files
*.slo
*.lo
*.o
*.obj

# Precompiled Headers
*.gch
*.pch

# Compiled Dynamic libraries
*.so
*.dylib
*.dll

# Fortran module files
*.mod
*.smod

# Compiled Static libraries
*.lai
*.la
*.a
*.lib

# Executables
*.exe
*.out
*.app

# LSP Cache
.ccls-cache
.cache
compile_commands.json

# Build Directory
build/
"#;

pub const CLANG_FORMAT_CONFIG: &str = r#"---
Language:        Cpp
BasedOnStyle:  Google
AccessModifierOffset: -2
AllowShortFunctionsOnASingleLine: Empty
AllowShortIfStatementsOnASingleLine: Never
AllowShortLoopsOnASingleLine: false
IncludeBlocks: Preserve
IndentCaseLabels: false
...
"#;

pub const CLANG_TIDY_CONFIG: &str = r#"---
Checks: '
        bugprone-*,
        clang-analyzer-*,
        cppcoreguidelines-*,
        modernize-*,
        performance-*,
        portability-*,
        readability-*,
        -cppcoreguidelines-avoid-magic-numbers,
        -modernize-use-nodiscard,
        -modernize-use-trailing-return-type,
        -readability-identifier-length,
        -readability-magic-numbers,
        -readability-qualified-auto
        '
"#;

pub const CMAKE_LISTS: &str = r#"cmake_minimum_required(VERSION 3.20)
project(
  {name}
  VERSION 0.1.0
  LANGUAGES CXX C
)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_BINARY_DIR}/cmake)

### Options
if(POLICY CMP0167)
  cmake_policy(SET CMP0167 NEW)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

### CPM
set(CPM_DOWNLOAD_VERSION "{cpm_version}")
set(CPM_HASH_SUM "{cpm_hash_sum}")
set(CPM_DOWNLOAD_URL "https://github.com/cpm-cmake/CPM.cmake/releases/download/v${CPM_DOWNLOAD_VERSION}/CPM.cmake")

if(CPM_SOURCE_CACHE)
  set(CPM_DOWNLOAD_LOCATION "${CPM_SOURCE_CACHE}/cpm/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
elseif(DEFINED ENV{CPM_SOURCE_CACHE})
  set(CPM_DOWNLOAD_LOCATION "$ENV{CPM_SOURCE_CACHE}/cpm/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
else()
  set(CPM_DOWNLOAD_LOCATION "${CMAKE_BINARY_DIR}/cmake/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
endif()

get_filename_component(CPM_DOWNLOAD_LOCATION ${CPM_DOWNLOAD_LOCATION} ABSOLUTE)

if (NOT EXISTS ${CPM_DOWNLOAD_LOCATION})
  file(DOWNLOAD ${CPM_DOWNLOAD_URL} ${CPM_DOWNLOAD_LOCATION}
       EXPECTED_HASH SHA256=${CPM_HASH_SUM})
endif()

include(${CPM_DOWNLOAD_LOCATION})

### Library
CPMAddPackage("gh:fmtlib/fmt#11.1.3")

### Executable
add_executable({name} src/main.cc)
target_link_libraries({name} PRIVATE fmt::fmt)
"#;

pub const MAIN_CC: &str = r#"#include <fmt/format.h>

int main() {
    fmt::print("Hello, world!\n");
    return 0;
}
"#;
