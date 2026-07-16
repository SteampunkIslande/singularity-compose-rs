use std::{
    collections::BTreeMap,
    env,
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::bail;
use clap::Parser;
use colored::{Color, Colorize};

use inquire::validator::MaxLengthValidator;
use is_root::is_root;

use minijinja::Environment;

mod cli;
mod datatypes;
mod error;
mod utils;

use cli::*;
use datatypes::*;
use utils::*;

struct UnitFile {
    file_name: PathBuf,
    file_content: String,
}

const YAML_COMPOSE_DIR: &str = "/etc/singularity-compose-rs";
const YAML_COMPOSE_FILE: &str = "/etc/singularity-compose-rs/compose.yaml";

/// Entry point function to start services
fn compose_up(up_command: UpCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let services = doc.services_for_groups(&up_command.groups);
    let service_names: Vec<String> = services
        .iter()
        .map(|s| format!("scompose-{}.service", s.service_name))
        .collect();

    if !service_names
        .iter()
        .all(|service_name| Path::new("/etc/systemd/system").join(service_name).exists())
    {
        bail!(
            "Cannot activate required services:\n{}\nSome files are missing. Please run `scompose build` to update service files.",
            services
                .iter()
                .map(|s| format!("- `{}`", s.service_name.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    systemctl_run(
        &service_names,
        SystemdCommand::Start,
        true,
        up_command.dry_run,
    )?;
    systemctl_run(
        &service_names,
        SystemdCommand::Enable,
        true,
        up_command.dry_run,
    )?;

    Ok(())
}

/// Entry point function to update unit files according to current content of `/etc/singularity-compose-rs/compose.yaml`
fn compose_build(build_command: BuildCommand, jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;
    let services = doc.services_for_groups(&build_command.groups);

    let unit_files = utils::render_unit_files(&services, jinja_env)?;
    let dry_run = build_command.dry_run;

    // Detect services that are being overwritten (existing) vs new.
    // Existing services with changed definitions must be stopped, disabled and removed.
    // New services just need their unit files created.
    let mut to_write: Vec<UnitFile> = Vec::new();
    for unit_file in unit_files.into_iter() {
        let service_name = unit_file.file_name.file_name().map(|name| {
            name.to_string_lossy()
                .trim_start_matches("scompose-")
                .trim_end_matches(".service")
                .to_string()
        });
        let Some(service_name) = service_name else {
            continue;
        };
        let unit_file_name = format!("scompose-{}.service", service_name);

        if unit_file.file_name.exists() {
            let existing = std::fs::read_to_string(&unit_file.file_name).unwrap_or_default();
            if existing == unit_file.file_content {
                // No change, skip
                continue;
            }
            eprintln!(
                "Service `{}` definition changed, regenerating its unit file.",
                service_name
            );
            if dry_run {
                eprintln!(
                    "Would stop, disable and remove overwritten unit file `{}`. Then write the updated version.",
                    unit_file_name
                );
            } else {
                systemctl_run(&[&unit_file_name], SystemdCommand::Stop, true, false)?;
                systemctl_run(&[&unit_file_name], SystemdCommand::Disable, true, false)?;
                if unit_file.file_name.exists() {
                    std::fs::remove_file(&unit_file.file_name)?;
                    eprintln!("Removed unit file: {}", unit_file.file_name.display());
                }
            }
        } else {
            eprintln!(
                "Service `{}` is new, generating its unit file.",
                service_name
            );
        }
        to_write.push(unit_file);
    }

    utils::write_unit_files(&to_write, dry_run)?;
    // When building, we only generate new unit files from services. But some unit files might be absent from `/etc/singularity-compose-rs/compose.yaml`. These files must be cleaned up.
    cleanup(definition_file, dry_run)?;
    daemon_reload()?;

    if !to_write.is_empty() {
        eprintln!(
            "{} service(s) had unit files generated. You may want to run `scompose up` to start them.",
            to_write.len()
        );
    }

    Ok(())
}

/// Entry point function to stop services
fn compose_down(down_command: DownCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let services = doc.services_for_groups(&down_command.groups);
    let service_names: Vec<_> = services
        .iter()
        .map(|s| format!("scompose-{}", s.service_name))
        .collect();
    systemctl_run(
        &service_names,
        SystemdCommand::Stop,
        false,
        down_command.dry_run,
    )?;
    Ok(())
}

/// Entry point function to list services that are defined in `/etc/singularity-compose-rs/compose.yaml`
fn compose_list(list_command: ListCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = if !definition_file.exists() {
        Document::default()
    } else {
        Document::try_from_file_path(definition_file)?
    };

    let services = doc.services_for_groups(&list_command.groups);
    if services.is_empty() {
        println!("No services found.");
        return Ok(());
    }

    let states = utils::query_scompose_unit_states()?;

    #[derive(Debug, Clone, Default)]
    struct ServiceNode {
        name: String,
        state: Option<utils::UnitState>,
    }

    #[derive(Debug, Default)]
    struct GroupNode {
        name: String,
        sub_groups: BTreeMap<String, GroupNode>,
        services: Vec<ServiceNode>,
    }

    impl GroupNode {
        fn new(name: String) -> Self {
            Self {
                name,
                sub_groups: BTreeMap::new(),
                services: Vec::new(),
            }
        }
    }

    fn insert_service(tree: &mut GroupNode, parts: &[&str], node: ServiceNode) {
        if parts.is_empty() {
            return;
        }
        // Cannot panic since if parts is empty, below code is unreachable
        let (first, rest) = parts.split_at(1);
        let entry = tree
            .sub_groups
            .entry(first[0].to_string())
            .or_insert_with(|| GroupNode::new(first[0].to_string()));
        if rest.is_empty() {
            entry.services.push(node);
        } else {
            insert_service(entry, rest, node);
        }
    }

    let mut root = GroupNode::new("all services".to_string());
    for service in &services {
        let node = ServiceNode {
            name: service.service_name.clone(),
            state: states.get(&service.service_name).cloned(),
        };
        if let Some(group) = &service.service_group {
            let parts: Vec<&str> = group.split('.').collect();
            insert_service(&mut root, &parts, node);
        } else {
            root.services.push(node);
        }
    }

    // Color a service node according to its systemd state.
    fn color_service(node: &ServiceNode) -> String {
        let state = match &node.state {
            None => "not loaded".to_string(),
            Some(state) => state.summary(),
        };
        let color = match &node.state {
            None => Color::BrightBlack,
            Some(state) => match state.active.as_str() {
                "active" => Color::Green,
                "inactive" => Color::Yellow,
                "failed" => Color::Red,
                _ => Color::White,
            },
        };
        format!(
            "{}  [{}]",
            node.name.color(color).bold(),
            state.color(color)
        )
    }

    // Renders the children of `group` (sub-groups first, then services) with proper
    // box-drawing connectors. The group's own name is printed by its parent loop (or the
    // caller for the root), so this function only draws descendants.
    fn render_group(group: &GroupNode, prefix: &str) {
        let mut child_groups: Vec<&GroupNode> = group.sub_groups.values().collect();
        child_groups.sort_by_key(|g| &g.name);
        let mut child_services = group.services.clone();
        child_services.sort_by(|a, b| a.name.cmp(&b.name));

        let total = child_groups.len() + child_services.len();
        let mut index = 0;
        for child in child_groups {
            index += 1;
            let last = index == total;
            let connector = if last { "└── " } else { "├── " };
            println!("{}{}{}", prefix, connector, child.name.bold());
            let child_prefix = format!("{}{}", prefix, if last { "    " } else { "│   " });
            render_group(child, &child_prefix);
        }
        for service in &child_services {
            index += 1;
            let last = index == total;
            let connector = if last { "└── " } else { "├── " };
            println!("{}{}{}", prefix, connector, color_service(service));
        }
    }

    println!("{}", root.name.bold());
    render_group(&root, "");

    Ok(())
}

/// Entry point function to add existing yaml to `/etc/singularity-compose-rs/compose.yaml`. This function also generates the appropriate unit files.
fn compose_add(add_command: AddCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    // Create dir if it doesn't exist yet
    ensure_etc_exists()?;
    let doc = if !definition_file.exists() {
        // For the add command, this makes sense: if file `/etc/singularity-compose-rs/compose.yaml` doesn't exist yet,
        // this is exactly why this function is run.
        Document::default()
    } else {
        Document::try_from_file_path(definition_file)?
    };
    let input_doc = Document::try_from_file_path(&add_command.file)?;

    let merge_result = doc.merge_document(input_doc);

    for service in &merge_result.overwritten {
        eprintln!(
            "Overwriting existing service since definition changed: `{}`",
            service.service_name
        );
    }

    let file = File::create(definition_file)?;

    // Save to file
    yaml_serde::to_writer(file, &Document::from(merge_result.clone()))?;

    let MergeResult {
        unchanged,
        added,
        overwritten,
    } = merge_result;

    if !overwritten.is_empty() {
        eprintln!(
            "Warning: The following services' definitions changed and they will be stopped, disabled, and their unit files will be removed:"
        );
        for service in overwritten.iter() {
            eprintln!("  - {}", service.service_name);
        }
        for service in overwritten.iter() {
            let unit_file_name = format!("scompose-{}.service", service.service_name);
            let unit_file_path = Path::new("/etc/systemd/system").join(&unit_file_name);

            systemctl_run(&[&unit_file_name], SystemdCommand::Stop, true, false)?;
            systemctl_run(&[&unit_file_name], SystemdCommand::Disable, true, false)?;

            if unit_file_path.exists() {
                std::fs::remove_file(&unit_file_path)?;
                eprintln!("Removed unit file: {}", unit_file_path.display());
            }
        }
        eprintln!("Now, these re-defined services will be added along with the newly added ones.");
        unit_files_from_services(
            &[overwritten.as_slice(), added.as_slice()].concat(),
            _jinja_env,
            false,
        )?;
    } else {
        unit_files_from_services(&added, _jinja_env, false)?;
    }
    daemon_reload()?;

    eprintln!(
        "Successfully merged compose file. {} service(s) added, {} overwritten, {} left unchanged.",
        added.len(),
        overwritten.len(),
        unchanged.len()
    );

    Ok(())
}

/// Entry point function to cleanly remove a service from `/etc/singularity-compose-rs/compose.yaml`.
fn compose_remove(remove_command: RemoveCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let mut doc = Document::try_from_file_path(definition_file)?;

    let removed = doc.remove_services(&remove_command.service_names);
    if removed.is_empty() {
        eprintln!("No services were removed.");
        return Ok(());
    }

    for service in &removed {
        let unit_file_name = format!("scompose-{}.service", service.service_name);
        let unit_file_path = Path::new("/etc/systemd/system").join(&unit_file_name);
        eprintln!("Removing service `{}`", service.service_name);

        systemctl_run(&[&unit_file_name], SystemdCommand::Stop, true, false)?;
        systemctl_run(&[&unit_file_name], SystemdCommand::Disable, true, false)?;

        if unit_file_path.exists() {
            std::fs::remove_file(&unit_file_path)?;
            eprintln!("Removed unit file: {}", unit_file_path.display());
        }
    }

    let file = File::create(definition_file)?;
    yaml_serde::to_writer(file, &doc)?;
    eprintln!("Updated compose file: {}", definition_file.display());

    daemon_reload()?;

    eprintln!("Successfully removed {} service(s).", removed.len());

    Ok(())
}

/// Entry point function to remove any orphan service. This is somehow redundant with the `build` command, except this one only removes orphan service files without generating new ones.
fn compose_clean() -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    cleanup(definition_file, false)?;
    daemon_reload()?;
    Ok(())
}

/// Entry point function when the CLI is used with no arguments. This is a service creation wizard that automatically defines and builds a unit file for a new service.
fn service_creation_wizard(jinja_env: Environment) -> anyhow::Result<()> {
    // Prompts the user for informations about the service, then adds it to `/etc/singularity-compose-rs/compose.yaml`, then builds the unit file.
    println!(
        "Welcome to the service creation wizard. This wizard will guide you through the definition of singularity instances as Linux services."
    );
    let service_name = inquire::Text::new("Please enter a name for the new service:")
        .with_validator(validate_service_name)
        .prompt()?;
    let description = inquire::Text::new(
        "Please enter a description for this service (144 characters maximum, press ESC to skip):",
    )
    .with_validator(MaxLengthValidator::new(144))
    .prompt_skippable()?;
    let user =
        inquire::Text::new("Please choose the name of the user this service should be run by:")
            .with_default("root")
            .with_validator(validate_user_name)
            .prompt_skippable()?;
    let group =
        inquire::Text::new("Please choose the name of the group this service should be run by:")
            .with_default("root")
            .with_validator(validate_group_name)
            .prompt_skippable()?;
    let mut volumes: Vec<String> = Vec::new();
    loop {
        if inquire::Confirm::new(&format!(
            "Do you want to add a volume mount ? You defined {} bind(s) so far.",
            volumes.len()
        ))
        .with_default(false)
        .prompt_skippable()?
        .is_some_and(|b| b)
        {
            let host_p =
                inquire::Text::new("Please enter a path on the host (either file or directory):")
                    .with_autocomplete(FilePathCompleter::default())
                    .with_validator(validate_path)
                    .prompt()?;
            let container_p = {
                let p = inquire::Text::new(&format!("Please enter a path inside the container that `{}` on the host should bind to (leave empty or press ESC if you want it to be the same):",host_p)).prompt_skippable()?;
                match p {
                    Some(p) if !p.is_empty() => Some(p),
                    Some(_) => None, // If p is empty, set to None
                    None => None,
                }
            };
            let ro = inquire::prompt_confirmation(format!(
                "Do you want this bind ({}) to be read-only?",
                host_p
            ))?;
            volumes.push(match (host_p, container_p, ro) {
                (host_p, Some(container_p), true) => format!("{host_p}:{container_p}:ro"),
                (host_p, Some(container_p), false) => format!("{host_p}:{container_p}"),
                (host_p, None, true) => format!("{host_p}:{host_p}:ro"),
                (host_p, None, false) => host_p.to_string(),
            })
        } else {
            break;
        }
    }
    println!(
        "{} bind(s) will be defined for service `{}`. Please note that these don't include default binds defined in `/etc/singularity/singularity.conf`",
        volumes.len(),
        service_name
    );
    let pidfile = inquire::Text::new(&format!("Please provide a path for the PIDFile.\nPath should not exist when service is not running, and the parent of the given path should be writable by user {}, group {}",user.as_deref().unwrap_or("root"),group.as_deref().unwrap_or("root"))).prompt_skippable()?;

    let image = Path::new(&inquire::Text::new("Please provide a path to the singularity image that will run as a service.\nPlease make sure it has been built with a `%startscript` and that the startscript runs in the foreground!").with_validator(validate_path).with_validator(validate_sif_file).with_autocomplete(FilePathCompleter::default()).prompt()?).canonicalize()?.display().to_string();

    let restart = inquire::Select::new(
        "Please choose one of the following reasons your service should restart:",
        vec![
            "always",
            "no",
            "on-success",
            "on-failure",
            "on-abnormal",
            "on-abort",
            "on-watchdog",
        ],
    )
    .prompt_skippable()?
    .map(String::from);

    let after = inquire::Text::new("Please enter the names of the services that this one should be run after (space-separated). You can press escape to skip this part")
    .prompt_skippable()?.and_then(|s|if s.is_empty(){None}else{Some(s)});

    let requires = inquire::Text::new("Please enter the names of the services that this one actually requires (space-separated). You can press escape to skip this part")
    .prompt_skippable()?.and_then(|s|if s.is_empty(){None}else{Some(s)});

    let service_group = inquire::Text::new("Please enter the name of the group hierarchy this service should be part of (hierarchy is dot-separated)").prompt_skippable()?;
    let service_name_clone = service_name.clone();

    let service = Service {
        service_name,
        description,
        user,
        group,
        volumes,
        pidfile,
        image,
        restart,
        after,
        requires,
        service_group,
    };

    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let mut doc: Document = datatypes::Document::try_from_file_path(definition_file)?;
    doc.services.push(service);

    unit_files_from_services(&doc.services, jinja_env, false)?;
    daemon_reload()?;

    // Save to file!
    let file = File::create(definition_file)?;
    yaml_serde::to_writer(file, &doc)?;

    println!(
        "Successfully created new service `{0}`!\nYou can run `scompose up` to start and enable it",
        service_name_clone
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut jinja_env = Environment::new();
    jinja_env.set_undefined_behavior(minijinja::UndefinedBehavior::SemiStrict);

    let service_template_str = String::from_utf8(include_bytes!("service_template.j2").to_vec())?;
    jinja_env.add_template("service_template.j2", &service_template_str)?;

    if env::args().len() <= 1 {
        // Run the service creation wizard
        if !is_root() {
            bail!("You must be root to create a service!");
        }
        service_creation_wizard(jinja_env)?;
        return Ok(());
    }

    let cli = Cli::parse();

    match cli.command {
        ComposeSubcommand::Down(down_command) => {
            if !is_root() {
                bail!("You must be root to take services down!")
            }
            compose_down(down_command, jinja_env)?;
        }
        ComposeSubcommand::Up(up_command) => {
            if !is_root() {
                bail!("You must be root to bring services up!")
            }
            compose_up(up_command, jinja_env)?;
        }
        ComposeSubcommand::List(list_command) => {
            compose_list(list_command, jinja_env)?;
        }
        ComposeSubcommand::Build(build_command) => {
            if !is_root() {
                bail!("You must be root to create new services!")
            }
            compose_build(build_command, jinja_env)?;
        }
        ComposeSubcommand::Add(add_command) => {
            if !is_root() {
                bail!("You must be root to create new services!")
            }
            compose_add(add_command, jinja_env)?;
        }
        ComposeSubcommand::Remove(remove_command) => {
            if !is_root() {
                bail!("You must be root to remove services!")
            }
            compose_remove(remove_command, jinja_env)?;
        }
        ComposeSubcommand::Clean => {
            if !is_root() {
                bail!("You must be root to remove services!")
            }
            compose_clean()?;
        }
    }
    Ok(())
}
