# cmk 设计文档

## 0. 文档范围

本文档描述 cmk 的整体架构。cmk 由两个子系统组成：

- **Toolchain 子系统**：管理 clang/LLVM 工具链的安装、切换、卸载与从源码自举构建。
  来源于原 `clangup` 项目，已并入 cmk。
- **Project 子系统**：CMake 项目的开发辅助（构建、格式化、lint）+ 依赖管理。
  来源于原 `cmk` 项目。

两子系统通过 `~/.cmk/` 共享状态，通过 `.cmk.toml` 的 `[toolchain]` 段衔接。

---

## 1. 目标与非目标

### 1.1 目标

1. **CMake 项目辅助**（cmk 现状沿袭）：`cmk new/run/build/build-tu/fmt/lint/init`、
   补全、CPM 包跟踪。
2. **Clang/LLVM 工具链管理**：安装、切换、卸载多个版本，工具链拆包为
   `toolchain` / `devel` / `tools-extra` 三个独立可分发的包，提供从源码自举构建
   的 builder。
3. **依赖管理**：项目级声明依赖，cmk 负责下载、build、install 到 per-project
   prefix，cmake 通过 `find_package` 命中。标准 CMake 包走内置模板，异构构建
   走项目内 `build.sh`。
4. **零侵入 cmake**：项目 `CMakeLists.txt` 不需要为 cmk 改动。`find_package`
   照常工作；删除 `.cmk-deps/` 即可退回手工维护。
5. **平台支持**：
   - Linux x86_64（baseline: EL7 / glibc 2.17）
   - Linux aarch64（baseline: EL8 / glibc 2.28）
   - macOS arm64
   - 不支持 Windows。
6. **实现语言** Rust，发布二进制按 EL7 baseline。

### 1.2 非目标

明确不做，且不在未来扩展计划里：

- 跨项目复用 dep install 产物（store / content-addressed prefix）
- 全局 dep recipe 仓库 / recipe 共享
- Dep 二进制缓存（本机或远端）
- ABI 矩阵 / triplet / feature 抽象
- 版本范围求解器（`>=1.0,<2.0` 等）
- Sandbox / hermetic build
- 与 vcpkg / Conan 互操作
- IDE / build system 配置接管（仅提供 PATH shim 和 `cmk toolchain exec`）
- Cross-compile（每个 host 装本地 native 工具链）
- Toolchain builder 不追求与 LLVM 上游 release 流程对齐，只产出 cmk 自己的 manifest

非目标的共同特征：一旦做，就是"重新发明 vcpkg"或"重新发明 Nix"。

### 1.3 与 CPM 现有路径的关系

cmk 现状是 CPM 风格：`cmk add owner/repo` → `~/.config/cmk/pkg.json` + 写
`CPMAddPackage` 到 `CMakeLists.txt`，依赖在项目 build dir 内被 FetchContent
拉下来跟项目一起编。

新引入的 `[deps]` 是另一条路径：依赖 build & install 到独立 prefix，项目通过
`find_package` 命中。两者**正交共存**：

| 场景 | 推荐 |
|---|---|
| header-only / 想跟项目同 flag 编 / 小库 | 继续 CPM |
| 异构构建系统（autotools、b2） | `[deps.custom]` |
| 大件 / 想 install 一次重用 / 给静态分析工具用 | `[deps.cmake]` 或 `[deps.custom]` |

`cmk add` / `cmk update` / `cmk pkg option` 现有的 CPM 路径保持不变。

---

## 2. 平台与基线

| Target              | Baseline | glibc | 备注                                        |
| ------------------- | -------- | ----- | ------------------------------------------- |
| `linux-x86_64`      | EL7      | 2.17  | 容器内 build；可能需 bootstrap 较新 zlib    |
| `linux-aarch64`     | EL8      | 2.28  | 容器内 build                                |
| `darwin-arm64`      | macOS 13 | —     | host 上 build；libc++/unwinder 用系统 SDK   |

baseline 的含义：发布的 toolchain 二进制依赖的最低 glibc 版本（Linux）/ 最低
macOS 版本。cmk 自身二进制同样按此 baseline 发布。

Rust 在 EL7 上的可行性：`x86_64-unknown-linux-gnu` tier-1 目标最低 glibc 即 2.17，
与 EL7 对齐；cmk 二进制默认按 gnu target 分发即可，并提供 musl 静态版本作为备选。

---

## 3. 全局路径布局

cmk 的全局状态分两个根目录，遵循职责分离：

- **`~/.config/cmk/`**：用户配置（XDG `$XDG_CONFIG_HOME/cmk/`）
- **`~/.cmk/`**：状态与 cache（toolchain install / shims / 项目 dep 源码 cache /
  builder cache / ccache）

### 3.1 配置目录 `~/.config/cmk/`

```
~/.config/cmk/
├── config.toml                      # 默认 toolchain registry、镜像
└── pkg.json                         # CPM 包跟踪索引（cmk add / cmk update 维护）
```

### 3.2 状态目录 `~/.cmk/`

```
~/.cmk/
├── installed.json                   # 已装 toolchain + 已装包 + sha256
├── current                          # 全局激活 toolchain（被 shim 读取）
├── toolchains/
│   ├── 18.1.8-linux-x86_64/         # 三包覆盖到同一 prefix
│   │   ├── bin/  lib/  include/  share/  libexec/
│   │   └── .cmk/                    # 元数据：已安装的包列表、manifest 副本
│   └── 18.1.8-darwin-arm64/
├── shims/                           # PATH 注入点，转发到 current
│   ├── clang  clang++  lld  ...
│   └── clang-tidy  clangd  ...      # tools-extra 的 shim 仅在该包已装时存在
├── manifests/cache/                 # 远端 manifest 缓存
├── downloads/                       # toolchain tarball 下载缓存（GC 候选）
├── cache/                           # 项目 dep 用的 cache
│   ├── tarball/<sha256>.tar.{gz,...}    # 源码 tarball
│   └── src/<source_sha256>/             # 解压结果
├── build-cache/                     # builder 中间产物（GC 候选）
├── ccache/                          # builder 共享 ccache
└── host-deps/                       # builder host 依赖产物（zlib 等）
    └── el7-x86/zlib-1.3.1/
```

**关键约定**：
- 配置（用户编辑）走 `~/.config/cmk/`，状态（cmk 维护、可重建）走 `~/.cmk/`。
  删 `~/.cmk/` 不丢用户配置，删 `~/.config/cmk/` 不丢已装 toolchain。
- Toolchain install 用版本-平台二元组寻址（`18.1.8-linux-x86_64`），不用 hash。
- Dep cache 仅缓存源码（content-addressed），install 完全 per-project。
- `~/.cmk/cache/` vs `~/.cmk/downloads/`：前者给项目 dep 用，后者给 toolchain 用，
  分开避免相互 GC 干扰。

### 3.3 Toolchain 切换

两条路径并存：

- **Shim**：`~/.cmk/shims` 加入 PATH。shim 是一个轻量分发器二进制，读取
  `~/.cmk/current` 或当前目录向上查找的 `.cmk-toolchain`，exec 到对应 toolchain。
- **`cmk toolchain exec <ver> -- <cmd>...`**：不依赖 PATH，主要给 CI/脚本使用。

### 3.4 项目级 toolchain 覆盖

两种来源：

1. `.cmk.toml` 的 `[toolchain] use = "..."`（cmk build 等命令读取）
2. `.cmk-toolchain` 文件（shim 解析时按 `cwd → 父目录` 链向上查找，命中即用）

环境变量 `CMK_TOOLCHAIN` 覆盖一切。

`.cmk.toml` 与 `.cmk-toolchain` 不强制一致 —— 前者用于 `cmk build`，后者用于
直接调 shim 的场景（编辑器、ad-hoc 命令）。

---

## 4. `.cmk.toml` Schema

完整示例：

```toml
schema = 2

# === 工具链（可选） ===
[toolchain]
use = "18.1.8"                  # cmk 管理的 LLVM 版本
# 替代写法：
# use = "system"                # 显式用 $CC/$CXX 或系统默认
# use = { path = "/opt/clang" } # 外部 prefix（不通过 cmk 管理）

# === 依赖：标准 cmake 包 ===
[deps.cmake]
fmt    = "10.2.1"
spdlog = { version = "1.13.0",
           options = { SPDLOG_FMT_EXTERNAL = "ON" },
           deps    = ["fmt"] }

# === 依赖：自定义构建（项目内 build.sh） ===
[deps.custom]
openssl  = "1.1.1m"
boost    = { version = "1.86.0",
             options = { libs = "chrono,context,filesystem" } }
jemalloc = { version = "5.3.0", platforms = ["linux"] }
foundationdb = { ref  = "release-7.4-emqx-fork",
                 deps = ["openssl", "boost", "jemalloc"] }

# === 现有：构建辅助 ===
[build]
default = "build/debug"

[fmt]
ignore = ["third_party/**", "*.pb.h"]

[lint]
ignore = ["third_party/**", "*.pb.h"]
warnings_as_errors = false
header_filter = "^(src|include)/"
extra_args = ["-quiet"]
```

### 4.1 废弃字段

旧 `.cmk.toml` 出现以下字段时，`cmk build` 报错并指向迁移文档，不静默忽略：

- `[vars]`
- `[env]` / `[env.macos]` / `[env.linux]`

这些原本是用户手工配 `${DEPS_INSTALL}` 路径接到 cmake 的胶水。`[deps]` 一旦
存在，cmk 内部自动算出对应的 env 注入，用户不再复述。

### 4.2 Schema 演进策略

`.cmk.toml` 顶层 `schema = 2`。未声明 = `schema = 1`（旧格式，仅过渡期）。
schema = 1 不识别 `[toolchain]` / `[deps.*]`。任何破坏性变更走 `schema = N+1`
显式版本号；非破坏只加 optional 字段。

### 4.3 字段定义

#### `[toolchain]`

```toml
use = "18.1.8"                  # cmk-managed LLVM
use = "system"                  # 系统编译器，等同于不写 [toolchain]
use = { path = "/opt/clang" }   # 外部 prefix，cmk 仅注入 CC/CXX 不管理
```

cmk 不会自动 `cmk toolchain install` 缺失的版本 —— 报错并提示用户。

#### `[deps.cmake]` 项

简短：
```toml
fmt = "10.2.1"
```

完整：
```toml
fmt = { version = "10.2.1",
        repo    = "fmtlib/fmt",          # 可省，默认从 ~/.config/cmk/pkg.json 查别名
        options = { FMT_INSTALL = "ON" },
        deps    = [],
        platforms = ["linux", "macos"] }
```

#### `[deps.custom]` 项

```toml
openssl = { version = "1.1.1m",            # 或 ref，二选一
            options = { ... },             # 透传为 CMK_OPT_* env
            deps    = [...],
            platforms = [...] }
```

`version` 用于 release tarball；`ref` 用于 git（`git clone --depth=1 -b <ref>`）。

`options` 不被 cmk 解释，序列化为 `CMK_OPT_<key>=<value>` 环境变量传给 build.sh。

`deps` 列出本 dep 在 `[deps.cmake]` + `[deps.custom]` 里的依赖名。cmk 据此拓扑
排序，循环依赖直接报错。

`platforms` 缺省 = `["linux", "macos"]`。当前 host 不在列表里则跳过该 dep。

---

## 5. CLI

### 5.1 项目辅助（沿用现有）

```
cmk new <name>
cmk run <target>
cmk build [--build <dir>] [--no-deps]
cmk build-tu <file>
cmk refresh [<dir>]
cmk fmt [--all|--staged|--unstaged|<path>]
cmk lint [--all|--staged|--unstaged|<path>] [--fix] [-W] [-i]
cmk init [-f]
cmk completions <shell>
```

行为变化：
- `cmk build` / `cmk run` / `cmk build-tu` 在 `.cmk.toml` 有 `[toolchain]` 时
  注入 `CC/CXX`，有 `[deps.*]` 时按需触发 `cmk deps install` 并注入
  `CMAKE_PREFIX_PATH` 等。
- `cmk build --no-deps`：跳过 `[deps.*]` 一致性检查与自动 install，假设
  `.cmk-deps/install/` 已就绪。给 CI 强制顺序、或排查 stamp 抖动用。
- `cmk refresh`：清 build dir 并重 cmake configure。当 `[deps.*]` 内容自上次
  install 以来变了时，refresh 自动先跑 `cmk deps install`（与 `cmk build`
  同样的触发逻辑）。
- `cmk fmt` / `cmk lint` 在 `.cmk.toml` 有 `[toolchain]` 时，优先用该 toolchain
  下的 `clang-format` / `clang-tidy`（前提是 `tools-extra` 包已装）。找不到再
  fallback 到 `which clang-format` / `which clang-tidy`。理由：项目用 toolchain
  X 编出来的 AST，应该用 X 配套的 tools-extra 分析，避免 clang/clang-tidy 版本
  错配引发的诊断噪声。
- `cmk init` 模板更新：默认包含 `[toolchain]` / `[deps.cmake]` / `[deps.custom]`
  注释块，不再写 `[vars]` / `[env]`。

### 5.2 CPM 包跟踪（沿用现有）

```
cmk add <owner/repo> [-p | --cmake]    # -p 与 --cmake 互斥
cmk get <name>
cmk update [-p] [-y]
cmk pkg option <name> KEY=VALUE...
```

`cmk add` 三态：
- 默认（无 flag）：仅写入全局 `~/.config/cmk/pkg.json`，不动项目
- `-p` / `--project`：额外写入 root `CMakeLists.txt` 的 `CPMAddPackage`（CPM 路径）
- `--cmake`：额外写入项目 `.cmk.toml` 的 `[deps.cmake]`（新路径）

`-p` 与 `--cmake` 互斥 —— 同一包不能既走 CPM 又走 `[deps.cmake]`，避免 cmake
配阶段两条路径冲突。

### 5.3 Toolchain 子组（吸收原 clangup CLI）

```
cmk toolchain install <version> [--components toolchain,devel,tools-extra]
cmk toolchain install --manifest <path-or-url>     # 离线/自定义
cmk toolchain remove  <version>
cmk toolchain list                                  # 已装
cmk toolchain list --available                      # registry 可用
cmk toolchain use    <version>                      # 写 ~/.cmk/current
cmk toolchain which  <bin>                          # 当前激活 toolchain 中该工具的路径
cmk toolchain exec   <version> -- <cmd>...
cmk toolchain gc [--keep N]                         # 清 downloads/+build-cache/，可选 evict 老 toolchain
cmk toolchain build  <version> [...builder flags]   # 见 §7.4
cmk toolchain publish <dir> --to <registry>         # 见 §7.4
```

### 5.4 Dep 子组

```
cmk deps install            # 解析 [deps.*]，按需 fetch+build+install 到 .cmk-deps/install
cmk deps clean              # rm -rf .cmk-deps/
cmk deps list               # 列出当前 prefix 包含的 dep 与版本（读 .cmk-meta.json）
cmk deps stamp              # 打印每个 dep 的 stamp 输入与当前 stamp，便于排查
```

### 5.5 全局缓存

```
cmk cache clear             # rm -rf ~/.cmk/cache/  (源码 cache)
cmk cache size              # 打印各类 cache 占用
```

---

## 6. Toolchain 子系统

### 6.1 拆包

按 install_prefix 内文件归属划分为三个独立包：

#### `toolchain`（运行时核心，~200–400 MB）

- `bin/`: `clang`, `clang++`, `clang-<major>`, `clang-cpp`, `lld`, `ld.lld`,
  `lld-link`, `llvm-ar`, `llvm-nm`, `llvm-objcopy`, `llvm-objdump`, `llvm-ranlib`,
  `llvm-strip`, `llvm-readelf`, `llvm-symbolizer`, `llvm-cov`, `llvm-profdata`,
  `llvm-config`
- `lib/clang/<v>/`: compiler-rt builtins、crt、sanitizer runtime、内置头
- `lib/libc++.a`, `lib/libc++abi.a`（仅 Linux；macOS 不带，使用系统 SDK）
- `include/c++/v1/`（仅 Linux）
- `lib/libLLVM-<v>.so`, `lib/libclang-cpp.so.<v>`（动态 LLVM 主体）
- `libexec/`, `share/clang/` 中运行时部分

#### `devel`（开发依赖，~1–2 GB）

- `include/{llvm,llvm-c,clang,clang-c,lld}/`
- `lib/libLLVM*.a`, `lib/libclang*.a`, `lib/liblld*.a`
- `lib/cmake/{llvm,clang,lld}/`

依赖 `toolchain`。下游若仅链接动态 libLLVM 不需要 devel；链接静态 LLVM
（如自定义工具）才需要。

#### `tools-extra`（~300–500 MB）

- `bin/`: `clang-tidy`, `clangd`, `clang-format`, `clang-apply-replacements`,
  `clang-include-cleaner`, `clang-query`, `clang-refactor`, `clang-rename`,
  `clang-doc`, `clang-change-namespace`, `clang-reorder-fields`,
  `find-all-symbols`, `modularize`, `pp-trace`
- 配套 python 脚本（`share/clang/`）

依赖 `toolchain`（动态链接 libLLVM/libclang-cpp，共享 `lib/clang/<v>/` 头与
sanitizer runtime）。

### 6.2 动态链接策略

LLVM 构建启用：
```
-DLLVM_BUILD_LLVM_DYLIB=ON
-DLLVM_LINK_LLVM_DYLIB=ON
-DCLANG_LINK_CLANG_DYLIB=ON
```

`clang`, `clang-tidy`, `clangd` 等所有工具链接动态 libLLVM-<v>.so /
libclang-cpp.so.<v>，减小总体积，并使 `tools-extra` 与 `toolchain` 之间的依赖
关系清晰：tools-extra 依赖 toolchain 提供的 dylib。

### 6.3 macOS 例外

macOS 不构建 / 不分发 libcxx、libcxxabi，使用系统 SDK 提供的 libc++ 和 unwinder。
`toolchain` 包不含 `libc++.a`、`libc++abi.a`、`include/c++/v1`。
`CLANG_DEFAULT_CXX_STDLIB`、`CLANG_DEFAULT_UNWINDLIB` 走 LLVM 默认值。

### 6.4 Manifest 与 Registry

#### Manifest

每个 release 一份，描述所有平台的所有包：

```toml
[release]
version = "18.1.8"
built   = "2026-04-20"

[platform.linux-x86_64]
baseline       = "el7"
host_glibc_min = "2.17"
builder_base   = "el7-x86-2026q1"
system_libcxx  = false
system_unwinder = false

[platform.linux-x86_64.packages.toolchain]
url    = "https://github.com/<org>/cmk-dist/releases/download/v18.1.8/clang-18.1.8-linux-x86_64-toolchain.tar.zst"
sha256 = "..."
size   = 312000000

[platform.linux-x86_64.packages.devel]
requires = ["toolchain"]
url      = "..."
sha256   = "..."

[platform.linux-x86_64.packages.tools-extra]
requires = ["toolchain"]
url      = "..."
sha256   = "..."

[platform.darwin-arm64]
baseline        = "macos-13"
system_libcxx   = true
system_unwinder = true
```

#### Registry trait

```rust
trait Registry {
    fn fetch_index(&self) -> Result<Index>;
    fn fetch_manifest(&self, ver: &str) -> Result<Manifest>;
    fn tarball_url(&self, ver: &str, plat: &str, pkg: &str) -> Url;
}

struct GithubReleases { repo: String }   // 一期实现
struct HttpMirror     { base: Url }       // 预留
```

**一期实现：`GithubReleases`**
- 索引：repo 中固定 tag（如 `index`）的 release 上传 `index.json`
- 单个版本的 manifest：作为该版本 release 的 asset `manifest.toml`
- tarball：作为该版本 release 的 asset

#### 配置（`~/.config/cmk/config.toml`）

```toml
toolchain_registries = [
  "github:my-org/cmk-dist",
  "https://mirror.internal.example.com/cmk",
]
```

按顺序 fallback。

### 6.5 Builder

#### 抽象层

```rust
struct Recipe {
    target: Target,                       // (os, arch, libc)
    baseline: Baseline,                   // El7 | El8 | MacOS13
    host: HostToolchain,
    bootstrap: Option<BootstrapStage>,    // None = 直接走 final
    final_build: FinalStage,
    compiler_rt_pass2: bool,              // sanitizers 二轮
    container: Option<ContainerSpec>,     // Linux 必填，macOS = None
    cxx_stdlib: CxxStdlib,                // Bundled | System
    unwinder: Unwinder,                   // Libgcc | System
}

enum HostToolchain {
    SystemGcc { gcc_prefix: PathBuf, version: String },
    Cmk       { version: String, path: PathBuf },
    External  { cc: PathBuf, cxx: PathBuf, sysroot: Option<PathBuf> },
}

struct ContainerSpec {
    image: String,
    runtime: ContainerRuntime,            // Docker | Podman 自动探测
    mounts: Vec<Mount>,
    env:    Vec<(String, String)>,
    user:   UserMapping,                  // 默认映射当前 uid/gid
    network: NetworkPolicy,               // None（构建期默认）
}
```

#### Stage 流转

```
if host.provides_libcxx() && host.min_glibc() <= baseline.glibc():
    skip bootstrap
    final_build 直接以 host 作为编译器
else:
    bootstrap: 用 host 构建 minimal clang+lld（runtimes 全关）
    final_build: 用 bootstrap 产物构建完整 LLVM
                 （含 libc++/libc++abi/compiler-rt builtins+crt）

if compiler_rt_pass2:
    compiler-rt sanitizers 第二轮（用 final 产物的 clang）

package: 按 toolchain / devel / tools-extra 切片打 tar.zst + sha256
```

#### Host 选择策略

```rust
fn pick_host(target: &Target, baseline: &Baseline) -> HostToolchain {
    // 1) 优先：cmk store 中匹配 target 的最新版本
    if let Some(v) = store.latest_for(target) {
        return Cmk(v);
    }
    // 2) 平台特化回退
    match (target.os, baseline) {
        (Linux, El7) => detect_devtoolset(11),
        (Linux, El8) => detect_gcc_toolset(11),
        (MacOS, _)   => External {
            cc: which("clang"),
            cxx: which("clang++"),
            sysroot: xcrun_sdk(),
        },
    }
}
```

容器化场景下默认行为：base image 内已固化一份 bootstrap clang 在
`/opt/cmk-base/bootstrap-clang`，作为 `HostToolchain::Cmk` 直接复用，跳过
bootstrap stage。仅 `--force-bootstrap` 才回到 stage0 gcc。

#### 容器约束

Linux recipe 强制容器化。`final_build` 与 `bootstrap` 必须在容器内执行；
`--no-container` 在 Linux 默认拒绝（仅 `--shell` 调试与 macOS 例外）。

容器 mount 布局（容器内视角）：

```
/src              ro    源码 tarball 解压
/build            rw    中间产物         host: ~/.cmk/build-cache/<recipe>/
/output           rw    最终 tarball     host: --output 指定
/cmk-host         ro    用作 host 的版本 host: ~/.cmk/toolchains/<v>
/host-deps        ro    zlib 等          host: ~/.cmk/host-deps/
/ccache           rw                      host: ~/.cmk/ccache/
```

容器执行约束：
- `--network=none`：源码与依赖在 host 预先下载校验后挂入。
- 非 root 运行：uid/gid 映射当前用户，避免产物 root 权限。
- `--read-only` rootfs，仅显式 rw mount。
- 镜像 digest 锁定后写入 build provenance。

#### 校验

`final_build` 启动前的 host 自检：
1. 用 host 编译最小 C++ 程序。
2. `readelf -V` 检查产生的二进制 GLIBC 符号版本上界 ≤ baseline。
3. 检查 host 提供的 libc++ ABI（若用 Cmk host）。

任何一项不符直接 fail，避免静默污染产物的 baseline 兼容性。

### 6.6 Builder 镜像管线

构建管线分两层：**base image 自举**（不常变） + **recipe 化的日常版本 build**。

#### Base image（每年/每个大 LLVM 主版本一次）

产出 `cmk-builder-base:<baseline>-<rev>` 镜像，包含：

| 组件                | 源                                 | 用途                                 |
| ------------------- | ---------------------------------- | ------------------------------------ |
| stage0 gcc          | devtoolset-11 / gcc-toolset-11 RPM | 仅 build 自身依赖；不进最终 PATH     |
| zlib                | 源码                               | 给 LLVM 与 deps 链接                 |
| zstd                | 源码                               | tarball 压缩 + LLVM 可选依赖         |
| openssl             | 源码                               | 给 python 用                         |
| python3 (≥ 3.8)     | 源码                               | LLVM/lit/cmake 脚本                  |
| cmake (≥ 3.20)      | 源码                               | LLVM 18+ 要求                        |
| ninja               | 源码                               |                                      |
| git                 | 源码                               | EL7 自带太老                         |
| ccache              | 源码                               | 后续 build 共享                      |
| **bootstrap clang** | 自举                               | 容器内默认 host toolchain            |

Image 内目录约定：

```
/opt/cmk-base/
├── gcc-toolset-11/          # stage0，仅自举期间使用
├── deps/
│   ├── bin/  lib/  include/
└── bootstrap-clang/         # HostToolchain::Cmk 默认指向
    ├── bin/  lib/  include/
    └── lib/libc++.a libc++abi.a libLLVM-<v>.so
```

环境：
```
PATH=/opt/cmk-base/bootstrap-clang/bin:/opt/cmk-base/deps/bin:$PATH
```

bootstrap-clang 用 rpath 自指，不依赖 LD_LIBRARY_PATH。

自举步骤（仓库 `builder-images/base/bootstrap.sh`）：

```
FROM centos:7  (or rockylinux:8 for arm)
1. yum install devtoolset-11 + 最小 build deps
2. scl enable devtoolset-11 -- 顺序 build deps：
     zlib → zstd → openssl → python3 → cmake → ninja → git → ccache
   全部 install 到 /opt/cmk-base/deps，所有版本+sha256 锁在 deps.lock
3. scl enable devtoolset-11 -- 跑 cmk 两阶段（用 stage0 gcc）：
     stage1: gcc11 + libstdc++  →  minimal clang
     stage2: stage1-clang + libc++/libc++abi/compiler-rt  →  自洽 clang
   install 到 /opt/cmk-base/bootstrap-clang
4. 删除 stage0 gcc 与中间 build dirs（缩 image）
5. 写入 /opt/cmk-base/manifest.json：
     base_rev, deps versions+sha256, bootstrap-clang version, build date
```

触发与发布：
- 仓库 workflow `build-base-image.yml`：手动触发或 `builder-images/base/**` 变更触发。
- 矩阵：`(el7, x86_64)` × `(el8, aarch64)`。
- 推送：`ghcr.io/<org>/cmk-builder-base:<baseline>-<arch>-<rev>`。

#### Recipe image（薄派生层）

```
FROM cmk-builder-base:<baseline>-<arch>-<rev>
+ 加 build entrypoint 脚本（解析 recipe，调用 cmake/ninja）
+ 加 recipe 特化的环境变量
```

仓库 `builder-images/recipe/Dockerfile.<baseline>-<arch>`。
Image tag：`ghcr.io/<org>/cmk-builder:<baseline>-<arch>-<rev>`。

#### CI workflow（`build-toolchain-release.yml`）

矩阵：

| os               | arch    | baseline |
| ---------------- | ------- | -------- |
| ubuntu-latest    | x86_64  | el7      |
| ubuntu-24.04-arm | aarch64 | el8      |
| macos-14         | arm64   | macos-13 |

Linux 行：
```
1. checkout cmk
2. cargo build --release（产物：cmk CLI）
3. 下载 LLVM 源码 tarball，sha256 校验
4. cmk toolchain build <version> \
     --target linux-x86_64 \
     --image ghcr.io/<org>/cmk-builder:el7-x86-<rev> \
     --output dist/
5. 生成 manifest.toml 片段
6. 上传 dist/*.tar.zst 与 manifest 片段为 workflow artifact
```

汇总作业：
```
needs: [linux-x86_64, linux-aarch64, darwin-arm64]
1. 收集所有 manifest 片段，合并为完整 manifest.toml
2. 合并 sha256 列表
3. gh release create v<version> dist/*.tar.zst manifest.toml
4. 更新 index.json 并 push 到 index release
```

macOS 行：
```
1. checkout
2. cargo build --release
3. cmk toolchain build <version> --target darwin-arm64 --no-container --output dist/
   ↑ 单阶段构建：runtimes 仅 compiler-rt，不做 libcxx/libcxxabi
4. 上传 artifact
```

### 6.7 Build provenance

每个 release 的 manifest.toml 嵌入：
- 上游 LLVM 源码 tarball URL + sha256
- builder base image digest（按 sha256:digest，不是 tag）
- recipe image digest
- cmk CLI git commit
- 所有 host-deps（zlib 等）版本

确保从 manifest 单条目就能复现构建。

---

## 7. Dep 子系统

### 7.1 项目目录布局

```
project/
├── .cmk.toml
├── .cmk/                          # git 跟踪
│   └── recipes/                   # [deps.custom] 的 build.sh 在这
│       ├── openssl/
│       │   └── build.sh
│       ├── boost/
│       │   └── build.sh
│       └── foundationdb/
│           ├── build.sh
│           └── patches/
│               └── 0ec02371-macos.patch
├── .cmk-deps/                     # gitignored
│   ├── build/<dep>-<version>/     # build 中间产物
│   ├── install/                   # 单一 install prefix
│   │   ├── bin/ lib/ include/ share/
│   │   └── .cmk-meta.json         # 当前 prefix 包含的 dep 清单
│   └── stamps/<dep>-<version>     # 增量 marker
└── CMakeLists.txt
```

`[deps.cmake]` 项**没有** recipe 目录 —— 用 cmk 内置 cmake 模板（见 §7.3）。

### 7.2 总流程

`cmk deps install`：

```
1. 解析 .cmk.toml，校验 [deps.*] 项与拓扑序
2. 解析 [toolchain]，得到 CC / CXX / 工具链 PATH 前缀
3. 为每个 dep 算 stamp_input：
     sha256( dep_kind          # "cmake" | "custom"
           + version_or_ref
           + canonical(options)
           + toolchain_id
           + (custom 时) sha256(build.sh) + sha256 each patch )
4. 拓扑序遍历每个 dep d:
     if exists(.cmk-deps/stamps/<d>-<v>) and content == stamp_input:
         continue
     fetch_and_extract(d) → ~/.cmk/cache/src/<source_sha256>/
     mkdir -p .cmk-deps/build/<d>-<v>
     run build_engine(d) with env:
         CMK_SRC      = ~/.cmk/cache/src/<source_sha256>/
         CMK_BUILD    = .cmk-deps/build/<d>-<v>/
         CMK_PREFIX   = .cmk-deps/install/
         CMK_JOBS     = <CMK_DEFAULT_JOBS>
         CMK_OPT_<k>  = <v> for each options entry
         CC / CXX / PATH from [toolchain]
     write .cmk-deps/stamps/<d>-<v>  ← stamp_input 内容
5. 更新 .cmk-deps/install/.cmk-meta.json
```

`cmk build`（已有命令的扩展）：

```
1. 如果 .cmk.toml 有 [deps.*] 段：
     diff stamps vs current [deps.*] → 决定是否触发 cmk deps install
2. 准备 cmake env：
     CC / CXX                            ← [toolchain]
     CMAKE_PREFIX_PATH                   ← .cmk-deps/install
     PKG_CONFIG_PATH                     ← .cmk-deps/install/lib/pkgconfig
                                          + .cmk-deps/install/lib64/pkgconfig
     LD_LIBRARY_PATH / DYLD_LIBRARY_PATH ← .cmk-deps/install/lib(+lib64 on linux)
3. 调用现有 cmk build 流程
```

### 7.3 Build engine

#### `[deps.cmake]` 内置模板

```bash
cmake -S "$CMK_SRC" -B "$CMK_BUILD" -GNinja \
      -DCMAKE_INSTALL_PREFIX="$CMK_PREFIX" \
      -DCMAKE_PREFIX_PATH="$CMK_PREFIX" \
      -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
      <-D for each options entry>
ninja -C "$CMK_BUILD" -j "$CMK_JOBS" install
```

模板固定，不可由用户配置。需要更复杂的 cmake 调用 → 改用 `[deps.custom]`。

#### `[deps.custom]` 调用 build.sh

```bash
bash .cmk/recipes/<dep>/build.sh
```

build.sh 自行使用注入的环境变量。例：

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$CMK_BUILD"
cp -a "$CMK_SRC"/. .
[ -d "$(dirname "$0")/patches" ] && \
    for p in "$(dirname "$0")"/patches/*.patch; do patch -p1 < "$p"; done

./config CFLAGS="-fPIC -O3" --prefix="$CMK_PREFIX" --openssldir="$CMK_PREFIX" \
    no-shared no-tests
make -j"$CMK_JOBS"
make install_sw
```

cmk 不规定 build.sh 的具体写法，只保证环境变量契约稳定。

### 7.4 Source 获取规则

`[deps.cmake]` 默认从 GitHub release tarball 拉：
```
https://github.com/<owner>/<repo>/archive/refs/tags/<tag>.tar.gz
```
`<owner>/<repo>` 从 `~/.config/cmk/pkg.json` 查别名（沿用现有 `cmk add` 机制），
`<tag>` 从 `version` 推（默认尝试 `<version>` 和 `v<version>`）。

`[deps.custom]` 默认行为同上，但 build.sh 可以完全忽略 `$CMK_SRC` 自己重新拉
（例如 fdb 需要 git clone 整个 history 而不是 tarball）。这种情况下 stamp 输入仍
按 `version`/`ref` 算，build.sh 自行保证幂等。

---

## 8. 仓库布局

合并后的 cargo workspace：

```
cmk/
├── Cargo.toml                        # workspace
├── crates/
│   ├── cmk-cli/                      # clap 子命令分发，main.rs
│   ├── cmk-core/                     # 跨子系统：错误类型、平台探测、版本/路径
│   ├── cmk-project/                  # CMake 项目辅助：cmake_ast, fmt, lint
│   ├── cmk-config/                   # .cmk.toml schema + 解析
│   ├── cmk-pkg/                      # CPM 包跟踪（~/.config/cmk/pkg.json）
│   ├── cmk-toolchain/                # toolchain install/use/which/exec/gc/shim 集成
│   ├── cmk-registry/                 # Registry trait + GithubReleases / HttpMirror
│   ├── cmk-builder/                  # Recipe、HostToolchain、容器调度、stage 执行
│   ├── cmk-deps/                     # [deps.cmake]/[deps.custom] 引擎
│   └── cmk-shim/                     # 轻量分发器二进制（独立可执行）
├── builder-images/
│   ├── base/
│   │   ├── Dockerfile.el7-x86
│   │   ├── Dockerfile.el8-arm
│   │   ├── bootstrap.sh
│   │   └── deps.lock                 # zlib/zstd/python/cmake/... 版本+sha256
│   └── recipe/
│       ├── Dockerfile.el7-x86        # FROM base，加 entrypoint
│       └── Dockerfile.el8-arm
├── ci/
│   ├── build-base-image.yml
│   ├── build-recipe-image.yml
│   └── build-toolchain-release.yml
├── docs/
│   └── design.md                     # 本文档
└── README.md
```

### 8.1 Crate 依赖图

```
cmk-cli
  ├── cmk-project   ── cmk-config
  ├── cmk-pkg       ── cmk-config
  ├── cmk-toolchain ── cmk-registry, cmk-core
  ├── cmk-builder   ── cmk-toolchain, cmk-registry
  └── cmk-deps      ── cmk-toolchain, cmk-config, cmk-pkg

cmk-shim (独立 binary，仅依赖 cmk-core)
```

### 8.2 二进制产物

- `cmk` —— 主 CLI（来自 cmk-cli）
- `cmk-shim` —— 轻量分发器，安装到 `~/.cmk/shims/`，硬链接成 `clang`/`clang++`/...

---

## 9. 实现里程碑

按依赖顺序：

| M | 内容 | 备注 |
|---|---|---|
| **M0** | Workspace 重构：cmk 改为多 crate；clangup 代码搬迁到对应 crate（cmk-toolchain / cmk-registry / cmk-builder / cmk-shim） | 机械迁移，无功能变化 |
| **M1** | 路径迁移：`~/.clangup/` → `~/.cmk/`；CLI 命名：`clangup install` → `cmk toolchain install`；删除 clangup binary | 用户面破坏性 |
| **M2** | Toolchain 子组在 cmk 下走通：install/list/use/which/remove/exec/gc | 复用 clangup 既有实现 |
| **M3** | Shim 集成 + activation；`.cmk-toolchain` 文件支持 | 复用 clangup 既有实现 |
| **M4** | `.cmk.toml` schema = 2；`[toolchain]` 段；`cmk build` 注入 CC/CXX | 项目侧首次受益 |
| **M5** | `[deps.cmake]` + 内置 cmake 模板 + 全局源码 cache + per-project install prefix + stamp 增量 | 首次能装 fmt/spdlog |
| **M6** | `cmk build` 自动触发 `cmk deps install`；env 注入 `CMAKE_PREFIX_PATH` 等 |  |
| **M7** | `[deps.custom]` + build.sh 调用约定 + `CMK_OPT_*` 透传 + patches 目录 | flowmq 风格场景 |
| **M8** | `cmk add --cmake` + 旧 `[vars]`/`[env]` 报错引导 + `cmk init` 模板更新 |  |
| **M9** | flowmq 实战迁移作为验收 | 外部项目 |
| **M10** | `cmk toolchain build` (Linux 容器化 builder) | 复用 clangup M3-M5 |
| **M11** | Base image + recipe image CI；`cmk toolchain publish`；发布管线 | 复用 clangup M4-M7 |
| **M12** | aarch64 / EL8 base image | 复用 clangup M6 |
| **M13** | provenance、`cmk toolchain gc` 强化、shim fast path benchmark |  |

M0-M3 是搬运 + 改名，不动设计。M4-M9 是新功能（dep 子系统）。M10-M13 是 builder
管线收尾，cmk 自身能用之前的预构建产物，所以不是阻塞项。

---

## 10. 设计纪律

为防止方案在未来扩展中漂移成 vcpkg / Nix，以下条目是**硬约束**，任何修改前要
先重读本节：

### Dep 子系统

1. **不引入跨项目 dep install 复用**。任何"全局 dep store"提议都要先证明
   per-project install 的实际痛点（用户数 / 项目数 / 单次 install 时长），
   否则拒绝。
2. **不引入全局 dep recipe 仓库**。recipe 是项目内 build.sh，复用形式只允许
   "用户自己 `cp -r .cmk/recipes/openssl 到下一个项目`"。
3. **`[deps.cmake]` 的内置模板不可参数化**。需要改 cmake 调用方式 → 转
   `[deps.custom]`。
4. **build.sh 接口只能加 env 变量，不能加配置文件 / DSL**。
5. **`[deps.*]` 项的 `version` 字段只接精确值**，不接范围 / 别名 / 通配符。
6. **不做 dep binary cache**。即使是本机也不做 —— ccache 已经覆盖大部分场景。
7. **不引入 sandbox**。host build，接受非确定性。

### Toolchain 子系统

8. **`cmk toolchain build` 始终容器化（Linux）**。除调试 (`--shell`) 与 macOS
   外不允许 `--no-container`。
9. **Container `--network=none`**。源码与依赖在 host 预先下载校验后挂入，不在
   容器内联网。
10. **Builder host 自检不可跳过**：编译最小程序 + readelf 检查 GLIBC 上界 +
    libc++ ABI 检查，任一失败直接 fail。
11. **Manifest 必须包含 build provenance**：源码 sha256、base image digest、
    recipe image digest、cmk git commit、host-deps 版本。
12. **Toolchain registry trait 不为单一实现优化**。`GithubReleases` 是一期实现，
    但接口必须能容纳 `HttpMirror`（已留口）。

### 通用

13. **Schema 演进只加 optional 字段**，破坏性变更走 `schema = N+1` 显式版本号。
14. **CLI 顶层命名空间不再扩展**。新增能力一律落到 `cmk toolchain` /
    `cmk deps` / `cmk cache` / `cmk pkg` 子组。
15. **`~/.cmk/` 路径布局变更要走 schema 版本**，不静默兼容旧路径（提供一次性
    migration 工具）。

违反任一条都意味着方案在向重型工具漂移。如果某个真实需求确实必须打破上述约束，
先开一份单独的 design 提案讨论破口的代价，不要在迭代中悄悄加。
