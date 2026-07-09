use std::{
    fs::File,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
};

use anyhow::bail;
use clap::Parser;

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
    if up_command.dry_run {
        eprintln!("Below are the commands that would be run.");
        eprintln!(
            "First, start.\nWould call `systemctl start {}`",
            service_names.join(" ")
        );
        eprintln!(
            "Then, enable.\nWould call `systemctl enable {}`",
            service_names.join(" ")
        )
    } else {
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
        let status = std::process::Command::new("systemctl")
            .arg("start")
            .args(&service_names)
            .status()?;
        match status.code() {
            Some(code) if !status.success() => eprintln!(
                "Process `systemctl start {}` exited with status code: {code}",
                service_names.join(" ")
            ),
            Some(_) => eprintln!("Successfully activated services."),
            None => eprintln!(
                "Process terminated by signal {}.",
                status.signal().unwrap_or(-1)
            ),
        }
        let status = std::process::Command::new("systemctl")
            .arg("enable")
            .args(&service_names)
            .status()?;
        match status.code() {
            Some(code) if !status.success() => eprintln!(
                "Process `systemctl enable {}` exited with status code: {code}",
                service_names.join(" ")
            ),
            Some(_) => eprintln!("Successfully enabled services."),
            None => eprintln!("Process terminated by signal"),
        }
    }
    Ok(())
}

/// Entry point function to update unit files according to current content of `/etc/singularity-compose-rs/compose.yaml`
fn compose_build(build_command: BuildCommand, jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;
    unit_files_from_services(
        doc.services_for_groups(&build_command.groups).as_slice(),
        jinja_env,
        build_command.dry_run,
    )?;
    // When building, we only generate new unit_files from services. But some unit files might be absent from `/etc/singularity-compose-rs/compose.yaml`. These files must be cleaned up.
    cleanup(&definition_file, build_command.dry_run)?;
    daemon_reload()?;

    Ok(())
}

/// Entry point function to stop services
fn compose_down(down_command: DownCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let services = doc.services_for_groups(&down_command.groups);
    let service_names: Vec<String> = services
        .iter()
        .map(|s| format!("scompose-{}.service", s.service_name))
        .collect();
    if down_command.dry_run {
        eprintln!(
            "This would call `systemctl stop {}`",
            service_names.join(" ")
        );
    } else {
        let status = std::process::Command::new("systemctl")
            .arg("stop")
            .args(&service_names)
            .status()?;
        match status.code() {
            Some(code) if !status.success() => eprintln!(
                "Process `systemctl stop {}` exited with status code: {code}",
                service_names.join(" ")
            ),
            Some(_) => eprintln!("Successfully stopped services."),
            None => eprintln!("Process terminated by signal"),
        }
    }
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

    #[derive(Debug, Clone)]
    struct GroupTreeNode {
        sub_groups: std::collections::BTreeMap<String, GroupTreeNode>,
        services: Vec<String>,
    }

    impl GroupTreeNode {
        fn new() -> Self {
            Self {
                sub_groups: std::collections::BTreeMap::new(),
                services: Vec::new(),
            }
        }
    }

    fn insert_service(tree: &mut GroupTreeNode, parts: &[&str], service_name: String) {
        if parts.is_empty() {
            return;
        }
        // Cannot panic since if parts is empty, below code is unreachable
        let (first, rest) = parts.split_at(1);
        let entry = tree
            .sub_groups
            .entry(first[0].to_string())
            .or_insert_with(GroupTreeNode::new);
        if rest.is_empty() {
            entry.services.push(service_name);
        } else {
            insert_service(entry, rest, service_name);
        }
    }

    fn build_treenodes(tree: &GroupTreeNode) -> Vec<text_trees::StringTreeNode> {
        let mut nodes = Vec::new();
        for (name, node) in &tree.sub_groups {
            let mut children: Vec<text_trees::StringTreeNode> = Vec::new();
            for svc in &node.services {
                children.push(text_trees::StringTreeNode::new(svc.clone()));
            }
            for child in build_treenodes(node) {
                children.push(child);
            }
            children.sort_by_key(|a| a.label());
            nodes.push(text_trees::StringTreeNode::with_child_nodes(
                name.clone(),
                children.into_iter(),
            ));
        }
        nodes
    }

    let mut groups_tree = GroupTreeNode::new();
    let mut no_group_services: Vec<String> = Vec::new();

    for service in services {
        if let Some(group) = &service.service_group {
            let parts: Vec<&str> = group.split('.').collect();
            insert_service(&mut groups_tree, &parts, service.service_name.clone());
        } else {
            no_group_services.push(service.service_name.clone());
        }
    }

    let mut all_children: Vec<text_trees::StringTreeNode> = Vec::new();
    for node in build_treenodes(&groups_tree) {
        all_children.push(node);
    }
    for svc in no_group_services {
        all_children.push(text_trees::StringTreeNode::new(svc));
    }
    all_children.sort_by_key(|a| a.label());

    let root = text_trees::StringTreeNode::with_child_nodes(
        "all services".to_string(),
        all_children.into_iter(),
    );

    let output = root.to_string_with_format(&text_trees::TreeFormatting::dir_tree(
        text_trees::FormatCharacters::box_chars(),
    ))?;
    println!("{}", output);

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

            let status = std::process::Command::new("systemctl")
                .arg("stop")
                .arg(&unit_file_name)
                .status()?;
            match status.code() {
                Some(code) if !status.success() => eprintln!(
                    "Warning: `systemctl stop {}` exited with status code: {}",
                    unit_file_name, code
                ),
                None => eprintln!(
                    "Warning: `systemctl stop {}` terminated by signal",
                    unit_file_name
                ),
                _ => {}
            }

            let status = std::process::Command::new("systemctl")
                .arg("disable")
                .arg(&unit_file_name)
                .status()?;
            match status.code() {
                Some(code) if !status.success() => eprintln!(
                    "Warning: `systemctl disable {}` exited with status code: {}",
                    unit_file_name, code
                ),
                None => eprintln!(
                    "Warning: `systemctl disable {}` terminated by signal",
                    unit_file_name
                ),
                _ => {}
            }

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

        let status = std::process::Command::new("systemctl")
            .arg("stop")
            .arg(&unit_file_name)
            .status()?;
        match status.code() {
            Some(code) if !status.success() => eprintln!(
                "Warning: `systemctl stop {}` exited with status code: {}",
                unit_file_name, code
            ),
            None => eprintln!(
                "Warning: `systemctl stop {}` terminated by signal",
                unit_file_name
            ),
            _ => {}
        }

        let status = std::process::Command::new("systemctl")
            .arg("disable")
            .arg(&unit_file_name)
            .status()?;
        match status.code() {
            Some(code) if !status.success() => eprintln!(
                "Warning: `systemctl disable {}` exited with status code: {}",
                unit_file_name, code
            ),
            None => eprintln!(
                "Warning: `systemctl disable {}` terminated by signal",
                unit_file_name
            ),
            _ => {}
        }

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
    cleanup(&definition_file, false)?;
    daemon_reload()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut jinja_env = Environment::new();

    jinja_env.set_undefined_behavior(minijinja::UndefinedBehavior::SemiStrict);

    let service_template_str = String::from_utf8(include_bytes!("service_template.j2").to_vec())?;
    jinja_env.add_template("service_template.j2", &service_template_str)?;

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
