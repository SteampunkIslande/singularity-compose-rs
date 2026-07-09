use crate::datatypes::Service;
use crate::{UnitFile, YAML_COMPOSE_DIR, datatypes};
use anyhow::{Context, bail};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use inquire::autocompletion::Replacement;
use inquire::validator::Validation::{self};
use inquire::{Autocomplete, CustomUserError};
use minijinja::{Environment, context};
use std::collections::HashSet;
use std::fmt::Debug;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};

/// Utility function to create the required directory `/etc/singularity-compose-rs`
pub fn ensure_etc_exists() -> anyhow::Result<()> {
    let d = Path::new(YAML_COMPOSE_DIR);
    if !d.is_dir() {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}

/// Utility function to (over)write unit files from a list of services
pub fn unit_files_from_services(
    services: &[Service],
    jinja_env: Environment,
    dry_run: bool,
) -> anyhow::Result<()> {
    let mut unit_files: Vec<UnitFile> = Vec::new();
    for service in services {
        let service_image = Path::new(&service.image);
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
    if dry_run {
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

/// Utility function to delete unit files that are no longer defined in `/etc/singularity-compose-rs`
pub fn cleanup(definition_file: &Path, dry_run: bool) -> anyhow::Result<()> {
    let known: HashSet<String> = datatypes::Document::try_from_file_path(definition_file)
        .context(
            "Le fichier `/etc/singularitycompose-rs/compose.yaml` ne peut pas être interprété.",
        )?
        .services
        .iter()
        .map(|s| format!("scompose-{}.service", s.service_name))
        .collect();

    let orphans: Vec<(PathBuf, String)> = glob::glob("/etc/systemd/system/scompose-*.service")
        .context("Invalid glob pattern, this is a developer's mistake")?
        .filter_map(Result::ok)
        .filter_map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .map(|name| (p, name))
        })
        .filter(|(_, name)| !known.contains(name))
        .collect();

    if orphans.is_empty() {
        eprintln!("Nothing to clean: no orphan service files found.");
        return Ok(());
    }

    if !dry_run {
        eprintln!("Found {} orphan services to remove.", orphans.len());
        for (path, name) in &orphans {
            eprintln!("Removing orphan service `{}`", name);

            systemctl_run(&[&name], SystemdCommand::Stop, true, false)?;
            systemctl_run(&[&name], SystemdCommand::Disable, true, false)?;

            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        eprintln!(
            "Clean complete: removed {} orphan service file(s).",
            orphans.len()
        );
    } else {
        eprintln!("{} orphan services found.", orphans.len());
        eprintln!(
            "The following services would be removed (once stopped and disabled): {}",
            orphans
                .iter()
                .map(|o| format!("service: `{}` - file: `{}`", o.1, o.0.display()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(())
}

/// Utility function to update systemd so it can forget deleted services and acknowledge new/changed ones.
pub fn daemon_reload() -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .arg("daemon-reload")
        .status()?;
    if let Some(code) = status.code() {
        if !status.success() {
            bail!(
                "Command `systemctl daemon-reload` exited with status {}",
                code
            );
        }
    } else {
        bail!(
            "Command `systemctl daemon-reload` was interrupted by signal {}",
            status.signal().unwrap_or(-1)
        );
    }
    Ok(())
}

#[derive(PartialEq)]
pub enum SystemdCommand {
    Start,
    Stop,
    Enable,
    Disable,
}

impl Debug for SystemdCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => write!(f, "start")?,
            Self::Stop => write!(f, "stop")?,
            Self::Enable => write!(f, "enable")?,
            Self::Disable => write!(f, "disable")?,
        }
        Ok(())
    }
}

/// Utility function to call systemd start|stop|enable|disable
pub fn systemctl_run<T: AsRef<str>>(
    service_names: &[T],
    systemd_command: SystemdCommand,
    ignore_non_zero_status: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    if service_names.is_empty() {
        if dry_run {
            eprintln!(
                "Would not run systemctl {:?} with no arguments...",
                systemd_command
            );
            return Ok(());
        } else {
            bail!(
                "Not running systemctl {:?} with no arguments!",
                systemd_command
            );
        }
    }
    let mut cmd = std::process::Command::new("systemctl");
    cmd.arg(match systemd_command {
        SystemdCommand::Start => "start",
        SystemdCommand::Stop => "stop",
        SystemdCommand::Enable => "enable",
        SystemdCommand::Disable => "disable",
    })
    .args(
        service_names
            .iter()
            .map(|s| String::from(s.as_ref()))
            .collect::<Vec<String>>(),
    );
    let cmd_str = format!("{:?}", cmd).replace("\"", "");
    if dry_run {
        eprintln!("Would run: `{}`", cmd_str);
        Ok(())
    } else {
        let status = cmd.status()?;
        match (status.code(), status.signal()) {
            (Some(code), _) if !status.success() => {
                eprintln!("Process `{}` exited with status code: {}", cmd_str, code);
                if !ignore_non_zero_status {
                    bail!("Running ̀{}` failed, this is considered an error.", cmd_str);
                }
            }
            (Some(_), _) => eprintln!(
                "Successfully {} services {}.",
                match systemd_command {
                    SystemdCommand::Start => "started",
                    SystemdCommand::Stop => "stopped",
                    SystemdCommand::Enable => "enabled",
                    SystemdCommand::Disable => "disabled",
                },
                service_names
                    .iter()
                    .map(|s| String::from(s.as_ref()))
                    .collect::<Vec<String>>()
                    .join(" ")
            ),
            (None, Some(signal)) => eprintln!("Process terminated by signal {}.", signal),
            (None, None) => eprintln!("Should be unreachable..."),
        }
        Ok(())
    }
}

pub fn validate_service_name(name: &str) -> Result<Validation, inquire::error::CustomUserError> {
    let doc = datatypes::Document::try_from_file_path(crate::YAML_COMPOSE_FILE)
        .context("Can't open compose file!")?;
    let is_valid = !doc.services.iter().any(|s| s.service_name == name);
    if is_valid {
        Ok(Validation::Valid)
    } else {
        Ok(Validation::Invalid("Service already exists!".into()))
    }
}

pub fn validate_user_name(name: &str) -> Result<Validation, inquire::error::CustomUserError> {
    match sysinfo::Users::new()
        .list()
        .iter()
        .find(|u| u.name() == name)
    {
        Some(_) => Ok(Validation::Valid),
        None => Ok(Validation::Invalid(
            format!("User {} does not exist!", name).into(),
        )),
    }
}

pub fn validate_group_name(name: &str) -> Result<Validation, CustomUserError> {
    match sysinfo::Groups::new()
        .list()
        .iter()
        .find(|g| g.name() == name)
    {
        Some(_) => Ok(Validation::Valid),
        None => Ok(Validation::Invalid(
            format!("Group {} does not exist!", name).into(),
        )),
    }
}

pub fn validate_path(path: &str) -> Result<Validation, CustomUserError> {
    Ok(if Path::new(path).exists() {
        Validation::Valid
    } else {
        Validation::Invalid("Path must exist!".into())
    })
}

pub fn validate_sif_file(path: &str) -> Result<Validation, CustomUserError> {
    let p = Path::new(path);
    if !p.exists() {
        Ok(Validation::Invalid(
            format!("{} does not exist", path).into(),
        ))
    } else {
        if let Some(ext) = p.extension() {
            if ext.display().to_string() == "sif" {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid(
                    format!("Expected sif file, found {}!", ext.display()).into(),
                ))
            }
        } else {
            Ok(Validation::Invalid(
                format!("{} should have .sif extension, found none!", path).into(),
            ))
        }
    }
}

/// This struct and its implementation details have been taken from https://github.com/mikaelmello/inquire/blob/main/examples/complex_autocompletion.rs
#[derive(Clone, Default)]
pub struct FilePathCompleter {
    input: String,
    paths: Vec<String>,
}

impl FilePathCompleter {
    fn update_input(&mut self, input: &str) -> Result<(), CustomUserError> {
        if input == self.input && !self.paths.is_empty() {
            return Ok(());
        }

        self.input = input.to_owned();
        self.paths.clear();

        let input_path = std::path::PathBuf::from(input);

        let fallback_parent = input_path
            .parent()
            .map(|p| {
                if p.to_string_lossy() == "" {
                    std::path::PathBuf::from(".")
                } else {
                    p.to_owned()
                }
            })
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let scan_dir = if input.ends_with('/') {
            input_path
        } else {
            fallback_parent.clone()
        };

        let entries = match std::fs::read_dir(scan_dir) {
            Ok(read_dir) => Ok(read_dir),
            Err(err) if err.kind() == ErrorKind::NotFound => std::fs::read_dir(fallback_parent),
            Err(err) => Err(err),
        }?
        .collect::<Result<Vec<_>, _>>()?;

        for entry in entries {
            let path = entry.path();
            let path_str = if path.is_dir() {
                format!("{}/", path.to_string_lossy())
            } else {
                path.to_string_lossy().to_string()
            };

            self.paths.push(path_str);
        }

        Ok(())
    }

    fn fuzzy_sort(&self, input: &str) -> Vec<(String, i64)> {
        let mut matches: Vec<(String, i64)> = self
            .paths
            .iter()
            .filter_map(|path| {
                SkimMatcherV2::default()
                    .smart_case()
                    .fuzzy_match(path, input)
                    .map(|score| (path.clone(), score))
            })
            .collect();

        matches.sort_by(|a, b| b.1.cmp(&a.1));
        matches
    }
}

impl Autocomplete for FilePathCompleter {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        self.update_input(input)?;

        let matches = self.fuzzy_sort(input);
        Ok(matches.into_iter().take(15).map(|(path, _)| path).collect())
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        self.update_input(input)?;

        Ok(if let Some(suggestion) = highlighted_suggestion {
            Replacement::Some(suggestion)
        } else {
            let matches = self.fuzzy_sort(input);
            matches
                .first()
                .map(|(path, _)| Replacement::Some(path.clone()))
                .unwrap_or(Replacement::None)
        })
    }
}
