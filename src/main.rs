use anyhow::Result;
use clap::Parser;

mod cmd;

#[derive(Debug, clap::Parser)]
#[command(version, about)]
struct Cli {
    #[clap(subcommand)]
    command: Option<SubCommand>,
    /// Run the default build command
    #[clap(short, long, value_name = "BUILD_DIR")]
    build: Option<String>,
    /// Run the default build command
    #[clap(short, long, default_value_t = false)]
    interactive: bool,
    /// Run the default build command
    #[clap(short, long)]
    jobs: Option<usize>,
    /// Run the default build command
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
    },
    /// Update the package index
    #[clap(name = "update", visible_alias = "u")]
    Update,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            SubCommand::Add { name } => cmd::exec_add(name).await,
            SubCommand::Update => cmd::exec_update().await,
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
                all,
                staged,
                unstaged,
                dry_run,
                verbose,
            } => cmd::exec_fmt(all, staged, unstaged, dry_run, verbose).await,
        }
    } else {
        cmd::exec_build(cli.target, cli.build, cli.interactive, cli.jobs).await
    }
}
