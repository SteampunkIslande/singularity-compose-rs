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

const YAML_COMPOSE_FILE: &str = "/etc/singularity-compose-rs/all-compose.yaml";

fn compose_up(up_command: UpCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let service_names: Vec<String> = doc
        .services
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
                doc.services
                    .iter()
                    .map(|s| format!("- ̀{}`", s.service_name.as_str()))
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

fn compose_build(build_command: BuildCommand, jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let mut unit_files: Vec<UnitFile> = Vec::new();
    for service in doc.services.iter() {
        let service_image = PathBuf::from_str(&service.image)?;
        if !service_image.exists() {
            bail!("Singularity image `{}` does not exist!", service.image);
        }
        if !service_image.is_absolute() {
            bail!(
                "Singularity image path should be absolute (found `{}`)",
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
        }
    }
    Ok(())
}

fn compose_down(down_command: DownCommand, _jinja_env: Environment) -> anyhow::Result<()> {
    let definition_file = Path::new(YAML_COMPOSE_FILE);
    let doc: Document = datatypes::Document::try_from_file_path(definition_file)?;

    let service_names: Vec<String> = doc
        .services
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
        ComposeSubcommand::List => {
            eprintln!(
                "{}",
                String::from_utf8(
                    std::process::Command::new("systemctl")
                        .arg("list-unit-files")
                        .arg("scompose-*")
                        .output()?
                        .stdout,
                )?
            );
        }
        ComposeSubcommand::Build(build_command) => {
            compose_build(build_command, jinja_env)?;
        }
    }
    Ok(())
}
