#[derive(clap::Subcommand, Debug, Clone)]
pub enum ComposeSubcommand {
    Build(BuildCommand),
    Up(UpCommand),
    Down(DownCommand),
    List,
}

/// (Re)-builds all the unit files
#[derive(clap::Parser, Debug, Clone)]
pub struct BuildCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Groups you want to (re)-build (comma-separated)
    ///
    /// Note that you can express a group hierarchy with `.`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

#[derive(clap::Parser, Debug, Clone)]
pub struct UpCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Groups you want to start (comma-separated)
    ///
    /// Note that you can express a group hierarchy with `.`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

/// Shuts down all the services that are defined in the singularity-compose.yaml file (or the file specified with --file).
#[derive(clap::Parser, Debug, Clone)]
pub struct DownCommand {
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Groups you want to shutdown (comma-separated)
    ///
    /// Note that you can express a group hierarchy with `.`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: ComposeSubcommand,
}
