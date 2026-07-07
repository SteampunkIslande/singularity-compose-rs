use std::{
    fs::File,
    io::Write,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;
use clap::Parser;

use minijinja::{Environment, context};

pub mod cli;
pub mod datatypes;
pub mod error;

use cli::*;
use datatypes::*;

struct UnitFile {
    file_name: PathBuf,
    file_content: String,
}

const YAML_COMPOSE_FILE: &str = "/etc/singularity-compose-rs/compose.yaml";

fn compose_up(up_command: UpCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let services = doc.services_for_groups(&up_command.groups);
    let service_names: Vec<String> = services
        .iter()
        .map(|s| format!("scompose-{}.service", s.service_name))
        .collect();
    if up_command.dry_run {
        eprintln!("Below is the command that would be run.");
    } else {
        if !service_names
            .iter()
            .all(|service_name| Path::new("/etc/systemd/system").join(service_name).exists())
        {
            bail!(
                "Cannot activate required services:\n{}\nSome files are missing. Please run `singularity-compose-rs build` to update service files.",
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
            Some(code) => eprintln!(
                "Process `systemctl enable {}` exited with status code: {code}",
                service_names.join(" ")
            ),
            None => eprintln!("Process terminated by signal"),
        }
    }
    Ok(())
}

//systemctl stop $service && systemctl disable $service && rm /etc/systemd/system/$service
fn compose_build(build_command: BuildCommand, jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let mut unit_files: Vec<UnitFile> = Vec::new();
    for service in doc.services_for_groups(&build_command.groups).iter() {
        let service_image = PathBuf::from_str(&service.image)?;
        if !service_image.exists() {
            bail!(
                "Singularity image `{}` does not exist! No unit files written.",
                service.image
            );
        }
        if !service_image.is_absolute() {
            bail!(
                "Singularity image path should be absolute (found `{}`) No unit files written.",
                service.image
            );
        }
        let unit_file_content =
            jinja_env
                .get_template("service_template.j2")?
                .render(context! {
                    service_name => service.service_name,
                    description => service.description,
                    user => service.user.as_deref().unwrap_or("root"),
                    group => service.group.as_deref().unwrap_or("root"),
                    binds => service.volumes.iter().map(|s|format!("-B {s}")).collect::<Vec<_>>().join(" "),
                    pidfile => service.pidfile.as_deref().unwrap_or(&format!("/run/{}.pid",service.service_name)),
                    image => service_image.display().to_string(),
                    restart => service.restart.as_deref().unwrap_or("always"),
                    after => service.after.as_deref().unwrap_or("network-online.target"),
                    requires => service.requires.as_deref().unwrap_or("network-online.target"),
                })?;
        unit_files.push(UnitFile {
            file_name: Path::new("/etc/systemd/system")
                .join(format!("scompose-{}.service", service.service_name)),
            file_content: unit_file_content,
        });
    }
    if build_command.dry_run {
        eprintln!("Below is what the generated unit files would look like.");
        for unit_file in unit_files.iter() {
            eprintln!(
                "File name: `{}`\n-----\nContent\n-----\n{}\n-----\n",
                unit_file.file_name.display(),
                unit_file.file_content
            );
        }
    } else {
        for unit_file in unit_files {
            eprintln!("Writing file `{}` ", unit_file.file_name.display());
            File::create(&unit_file.file_name)?.write_all(unit_file.file_content.as_bytes())?;
            eprintln!("Wrote file `{}`", unit_file.file_name.display());
        }
    }
    Ok(())
}

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
            Some(code) => eprintln!(
                "Process `systemctl stop {}` exited with status code: {code}",
                service_names.join(" ")
            ),
            None => eprintln!("Process terminated by signal"),
        }
    }
    Ok(())
}

fn compose_list(list_command: ListCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

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
            children.sort_by(|a, b| a.label().cmp(&b.label()));
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
    all_children.sort_by(|a, b| a.label().cmp(&b.label()));

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut jinja_env = Environment::new();

    jinja_env.set_undefined_behavior(minijinja::UndefinedBehavior::SemiStrict);

    let service_template_str = String::from_utf8(include_bytes!("service_template.j2").to_vec())?;
    jinja_env.add_template("service_template.j2", &service_template_str)?;

    match cli.command {
        ComposeSubcommand::Down(down_command) => {
            compose_down(down_command, jinja_env)?;
        }
        ComposeSubcommand::Up(up_command) => {
            compose_up(up_command, jinja_env)?;
        }
        ComposeSubcommand::List(list_command) => {
            compose_list(list_command, jinja_env)?;
        }
        ComposeSubcommand::Build(build_command) => {
            compose_build(build_command, jinja_env)?;
        }
    }
    Ok(())
}
