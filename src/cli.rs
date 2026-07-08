use std::path::PathBuf;

#[derive(clap::Subcommand, Debug, Clone)]
pub enum ComposeSubcommand {
    Build(BuildCommand),
    Up(UpCommand),
    Down(DownCommand),
    List(ListCommand),
    Add(AddCommand),
    Remove(RemoveCommand),
    /// Removes scompose unit files that are no longer defined in `/etc/singularity-compose-rs/compose.yaml`.
    ///
    /// This ensures `/etc/systemd/system` doesn't contain any `scompose-*` file that does not appear in `/etc/singularity-compose-rs/compose.yaml`.
    /// This shouldn't happen if you're using the proper method of calling `scompose remove <service name>` to remove a service, instead of directly editing the compose file.
    Clean,
}

/// (Re)-builds all the unit files.
///
#[derive(clap::Parser, Debug, Clone)]
pub struct BuildCommand {
    /// Will not write any unit files, only print
    ///
    /// This will print every file name, and every file content of all the unit files that would
    /// be written without this flag.
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Groups you want to (re)-build (comma-separated)
    ///
    /// Note that you can express a group hierarchy with `.`.
    /// If omitted, this will build all services defined in `/etc/singularity-compose-rs/compose.yaml`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

/// Starts either all defined services, or only those matching the `-g/--groups` filter.
///
/// Activates all the services that are defined in `/etc/singularity-compose-rs/compose.yaml` (or the ones specified with --groups).
#[derive(clap::Parser, Debug, Clone)]
pub struct UpCommand {
    /// Will not start any service, only print
    ///
    /// This will print the systemctl command that would be run without this flag.
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Groups you want to start
    ///
    /// Note that you can express a group hierarchy with `.`.
    /// If omitted, this will build all services defined in `/etc/singularity-compose-rs/compose.yaml`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

/// Stops either all defined services, or only those matching the `-g/--groups` filter.
///
/// Shuts down all the services that are defined in `/etc/singularity-compose-rs/compose.yaml` (or the ones specified with --groups).
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

/// Lists all services defined in `/etc/singularity-compose-rs/compose.yaml`. Displays them as a tree.
#[derive(clap::Parser, Debug, Clone)]
pub struct ListCommand {
    /// Groups you want to list (comma-separated)
    ///
    /// Note that you can express a group hierarchy with `.`
    #[arg(long = "groups", short = 'g', value_parser, num_args = 0.., value_delimiter = ',')]
    pub groups: Vec<String>,
}

/// Merge a compose file into the existing one and (re)-builds.
///
/// This command only stops/disables/overwrites services that are re-defined in the input file.
/// There is no dry run mode for this command, use with caution!
#[derive(clap::Parser, Debug, Clone)]
pub struct AddCommand {
    /// YAML file to merge into the existing compose file.
    ///
    /// Newly defined services will be added to `/etc/singularity-compose-rs/compose.yaml`.
    /// Services that were already defined in `/etc/singularity-compose-rs/compose.yaml` will be overwritten if different.
    #[arg(required = true)]
    pub file: PathBuf,
}

/// Remove one or more services from the compose file and stop/disable their unit files.
#[derive(clap::Parser, Debug, Clone)]
pub struct RemoveCommand {
    /// Service names to remove (one or more)
    #[arg(required = true, num_args = 1..)]
    pub service_names: Vec<String>,
}

#[derive(clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub command: ComposeSubcommand,
}
