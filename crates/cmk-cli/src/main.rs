use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::Shell;

mod cmd;

#[derive(Debug, clap::Parser)]
#[command(version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Option<SubCommand>,
    /// Build dir override (for the implicit `cmk` → `cmk build` shortcut)
    #[clap(short, long, value_name = "BUILD_DIR")]
    build: Option<String>,
    /// Pick the build target interactively via fzf
    #[clap(short, long, default_value_t = false)]
    interactive: bool,
    /// Number of parallel build jobs
    #[clap(short, long)]
    jobs: Option<usize>,
    /// Specific target name to build
    #[clap(short, long)]
    target: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
enum SubCommand {
    /// Add a package to the package index
    #[clap(name = "add", visible_alias = "a")]
    Add {
        /// The name of the package with the format of "owner/repo"
        name: String,
        /// Also insert `CPMAddPackage("gh:owner/repo#vTAG")` into the root
        /// CMakeLists.txt. Comments and formatting are preserved.
        #[clap(short, long)]
        project: bool,
    },
    /// Update the package index
    #[clap(name = "update", visible_alias = "u")]
    Update {
        /// Also scan the project's root CMakeLists.txt for CPMAddPackage calls
        /// and bump pinned versions to the latest tag found on GitHub.
        #[clap(short, long)]
        project: bool,
        /// Skip the confirmation prompt before applying project edits
        #[clap(short, long)]
        yes: bool,
    },
    /// Get the cached release of a package in the package index
    #[clap(name = "get", visible_alias = "g")]
    Get {
        /// The name or alias of the package
        name: String,
    },
    /// Create a new project
    #[clap(name = "new", visible_alias = "n")]
    New {
        /// The name of the project
        name: String,
        /// The name of the template to use
        #[clap(short, long)]
        template: Option<String>,
    },
    /// Run the executable target
    #[clap(name = "run", visible_alias = "r")]
    Run {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// The name of the executable target
        #[clap(short, long)]
        target: Option<String>,
        /// The arguments to pass to the executable target
        #[clap(last = true)]
        args: Vec<String>,
    },
    /// Build the project
    #[clap(name = "build", visible_alias = "b")]
    Build {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// Select the target to build interactively. When the target is
        /// specified, this option is ignored.
        #[clap(short, long, default_value_t = false)]
        interactive: bool,
        /// Run n jobs in parallel
        #[clap(short, long)]
        jobs: Option<usize>,
        /// The name of the executable target
        target: Option<String>,
    },
    /// Build the translation unit
    #[clap(name = "build-tu", visible_alias = "tu")]
    BuildTU {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// The name of the translation unit
        name: Option<String>,
    },
    /// Refresh the CMake build directory
    #[clap(name = "refresh", visible_alias = "ref")]
    Refresh {
        /// The path to the build directory relative to the project root
        build: Option<String>,
    },
    /// Format source files with clang-format
    #[clap(name = "fmt", visible_alias = "f")]
    Fmt {
        /// Format a single source file (path relative to PWD or absolute).
        /// Skips git-based selection.
        #[clap(conflicts_with_all = ["all", "staged", "unstaged"])]
        file: Option<String>,
        /// Format all tracked source files
        #[clap(short, long, conflicts_with_all = ["staged", "unstaged"])]
        all: bool,
        /// Format only staged files
        #[clap(short, long, conflicts_with_all = ["all", "unstaged"])]
        staged: bool,
        /// Format only unstaged files
        #[clap(short, long, conflicts_with_all = ["all", "staged"])]
        unstaged: bool,
        /// Print files that would be formatted without modifying them
        #[clap(short, long)]
        dry_run: bool,
        /// Print verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Generate shell completions to stdout
    #[clap(name = "completions")]
    Completions {
        /// The shell to generate completions for
        shell: Shell,
    },
    /// Scaffold a `.cmk.toml` in the project root
    #[clap(name = "init")]
    Init {
        /// Overwrite an existing `.cmk.toml`
        #[clap(short, long)]
        force: bool,
    },
    /// Manage CPM dependencies in the project's CMakeLists.txt
    #[clap(name = "pkg")]
    Pkg {
        #[clap(subcommand)]
        cmd: PkgCmd,
    },
    /// Lint source files with clang-tidy
    #[clap(name = "lint", visible_alias = "l")]
    Lint {
        /// The path to the build directory relative to the project root
        #[clap(short, long)]
        build: Option<String>,
        /// Lint a single source file (path relative to PWD or absolute).
        /// Skips git-based selection.
        #[clap(conflicts_with_all = ["all", "staged", "unstaged", "interactive"])]
        file: Option<String>,
        /// Pick a single source file from compile_commands.json interactively
        #[clap(short, long, conflicts_with_all = ["all", "staged", "unstaged"])]
        interactive: bool,
        /// Lint all tracked source files
        #[clap(short, long, conflicts_with_all = ["staged", "unstaged"])]
        all: bool,
        /// Lint only staged files
        #[clap(short, long, conflicts_with_all = ["all", "unstaged"])]
        staged: bool,
        /// Lint only unstaged files
        #[clap(short, long, conflicts_with_all = ["all", "staged"])]
        unstaged: bool,
        /// Apply suggested fixes (forces serial execution)
        #[clap(long)]
        fix: bool,
        /// Treat warnings as errors (overrides .cmk.toml)
        #[clap(short = 'W', long)]
        warnings_as_errors: bool,
        /// Bypass the per-file result cache
        #[clap(long)]
        no_cache: bool,
        /// Print verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Manage clang/LLVM toolchains (install, switch, build, etc.)
    #[clap(name = "toolchain")]
    Toolchain {
        #[clap(subcommand)]
        cmd: ToolchainCmd,
    },
    /// Manage project dependencies declared under `[deps.*]` (M5+).
    #[clap(name = "deps")]
    Deps {
        #[clap(subcommand)]
        cmd: DepsCmd,
    },
    /// Manage cmk's global cache directories.
    #[clap(name = "cache")]
    Cache {
        #[clap(subcommand)]
        cmd: CacheCmd,
    },
}

#[derive(Debug, clap::Subcommand)]
enum PkgCmd {
    /// Set OPTIONS for a CPM dependency in the root CMakeLists.txt.
    /// Rewrites the matching `CPMAddPackage` URI call into the keyword form
    /// with the requested OPTIONS appended.
    #[clap(name = "option")]
    Option {
        /// Package: repo basename (e.g. `fmt`), `owner/repo`, or alias from
        /// the global package index.
        name: String,
        /// One or more `KEY=VALUE` option pairs.
        #[clap(required = true)]
        opts: Vec<String>,
    },
}

#[derive(Debug, clap::Subcommand)]
enum ToolchainCmd {
    /// Install a release (from a registry, or a local manifest).
    Install {
        /// Release version (e.g. `18.1.8`). Required unless --manifest given.
        version: Option<String>,
        /// Comma-separated subset of toolchain,devel,tools-extra. Default: all.
        #[arg(long)]
        components: Option<String>,
        /// Manifest path or http(s):// / file:// URL (bypasses registry).
        #[arg(long)]
        manifest: Option<String>,
    },
    /// Remove an installed version.
    Remove { version: String },
    /// List installed versions, or registry-available with --available.
    List {
        #[arg(long)]
        available: bool,
    },
    /// Set the active version (writes ~/.cmk/current).
    Use { version: String },
    /// Print the path of <bin> in the active version.
    Which { bin: String },
    /// Run a binary from a specific version: `cmk toolchain exec 18.1.8 -- clang foo.c`.
    Exec {
        version: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
    /// Garbage-collect downloads/ + build-cache/, and (with --keep N) drop
    /// all installed versions except the N most recently installed.
    Gc {
        #[arg(long)]
        keep: Option<usize>,
    },
    /// Build a release from source via Recipe (M10+).
    Build {
        version: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        image: Option<String>,
        #[arg(long, value_enum)]
        runtime: Option<RuntimeArg>,
        #[arg(long)]
        no_bootstrap: bool,
        #[arg(long)]
        force_bootstrap: bool,
        #[arg(long)]
        no_container: bool,
        #[arg(long)]
        shell: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        /// LLVM source (URL or local llvm-project root). Default: upstream tarball.
        #[arg(long)]
        source: Option<String>,
    },
    /// Publish a build directory to a registry (M11+).
    Publish {
        dir: PathBuf,
        #[arg(long)]
        to: String,
    },
}

#[derive(Copy, Clone, Debug, clap::ValueEnum)]
pub enum RuntimeArg {
    Docker,
    Podman,
}

#[derive(Debug, clap::Subcommand)]
enum DepsCmd {
    /// Resolve `[deps.*]` and ensure `.cmk-deps/install/` is up-to-date.
    Install,
    /// Remove `.cmk-deps/`.
    Clean,
    /// List dependencies recorded in the current view.
    List,
    /// Print stamp inputs vs. recorded stamps for each dep.
    Stamp,
}

#[derive(Debug, clap::Subcommand)]
enum CacheCmd {
    /// Remove `~/.cmk/cache/` (project-dep source cache).
    Clear,
    /// Print sizes of cmk's cache directories.
    Size,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            SubCommand::Add { name, project } => cmd::exec_add(name, project).await,
            SubCommand::Update { project, yes } => cmd::exec_update(project, yes).await,
            SubCommand::Get { name } => cmd::exec_get(name).await,
            SubCommand::New { name, template } => cmd::exec_new(name, template).await,
            SubCommand::Run {
                target,
                args,
                build,
            } => cmd::exec_run(target, args, build).await,
            SubCommand::Build {
                target,
                build,
                interactive,
                jobs,
            } => cmd::exec_build(target, build, interactive, jobs).await,
            SubCommand::BuildTU { name, build } => cmd::exec_build_tu(name, build).await,
            SubCommand::Refresh { build } => cmd::exec_refresh(build).await,
            SubCommand::Fmt {
                file,
                all,
                staged,
                unstaged,
                dry_run,
                verbose,
            } => cmd::exec_fmt(file, all, staged, unstaged, dry_run, verbose).await,
            SubCommand::Completions { shell } => {
                let mut cmd = Cli::command();
                let name = cmd.get_name().to_string();
                clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
                Ok(())
            }
            SubCommand::Init { force } => cmd::exec_init(force).await,
            SubCommand::Pkg { cmd } => match cmd {
                PkgCmd::Option { name, opts } => cmd::exec_pkg_option(name, opts).await,
            },
            SubCommand::Lint {
                build,
                file,
                interactive,
                all,
                staged,
                unstaged,
                fix,
                warnings_as_errors,
                no_cache,
                verbose,
            } => {
                cmd::exec_lint(
                    build,
                    file,
                    interactive,
                    all,
                    staged,
                    unstaged,
                    fix,
                    warnings_as_errors,
                    no_cache,
                    verbose,
                )
                .await
            }
            SubCommand::Toolchain { cmd } => dispatch_toolchain(cmd),
            SubCommand::Deps { cmd } => dispatch_deps(cmd),
            SubCommand::Cache { cmd } => dispatch_cache(cmd),
        }
    } else {
        cmd::exec_build(cli.target, cli.build, cli.interactive, cli.jobs).await
    }
}

fn dispatch_toolchain(c: ToolchainCmd) -> Result<()> {
    use cmd::toolchain;
    match c {
        ToolchainCmd::Install {
            version,
            components,
            manifest,
        } => toolchain::install::run(version, components, manifest),
        ToolchainCmd::Remove { version } => toolchain::remove::run(&version),
        ToolchainCmd::List { available } => toolchain::list::run(available),
        ToolchainCmd::Use { version } => toolchain::use_::run(&version),
        ToolchainCmd::Which { bin } => toolchain::which::run(&bin),
        ToolchainCmd::Exec { version, rest } => toolchain::exec::run(&version, &rest),
        ToolchainCmd::Gc { keep } => toolchain::gc::run(keep),
        ToolchainCmd::Build {
            version,
            target,
            host: _,
            image,
            runtime,
            no_bootstrap: _,
            force_bootstrap: _,
            no_container,
            shell,
            output,
            source,
        } => toolchain::build::run(toolchain::build::BuildArgs {
            version,
            target,
            no_container,
            output,
            source,
            image,
            runtime,
            shell,
        }),
        ToolchainCmd::Publish { .. } => {
            anyhow::bail!("`cmk toolchain publish` lands in M11 (release pipeline)")
        }
    }
}

fn dispatch_deps(_c: DepsCmd) -> Result<()> {
    anyhow::bail!("`cmk deps` lands in M5+; see docs/design.md §7")
}

fn dispatch_cache(_c: CacheCmd) -> Result<()> {
    anyhow::bail!("`cmk cache` lands alongside `cmk deps` in M5+")
}
