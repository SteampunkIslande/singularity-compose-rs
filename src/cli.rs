#[derive(clap::Subcommand, Debug, Clone)]
pub enum ComposeSubcommand {
    Build(BuildCommand),
    Up(UpCommand),
    Down(DownCommand),
    List,
}

#[derive(clap::Parser, Debug, Clone)]
pub struct BuildCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,
}

#[derive(clap::Parser, Debug, Clone)]
pub struct UpCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,
}

/// Shuts down all the services that are defined in the singularity-compose.yaml file (or the file specified with --file).
#[derive(clap::Parser, Debug, Clone)]
pub struct DownCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: ComposeSubcommand,
}
