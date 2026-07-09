use crate::datatypes::Service;
use crate::{UnitFile, YAML_COMPOSE_DIR, datatypes};
use anyhow::{Context, bail};
use minijinja::{Environment, context};
use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
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

            let status = std::process::Command::new("systemctl")
                .arg("stop")
                .arg(name)
                .status()?;
            match status.code() {
                Some(code) if !status.success() => eprintln!(
                    "Warning: `systemctl stop {}` exited with status code: {}",
                    name, code
                ),
                None => eprintln!("Warning: `systemctl stop {}` terminated by signal", name),
                _ => {}
            }

            let status = std::process::Command::new("systemctl")
                .arg("disable")
                .arg(name)
                .status()?;
            match status.code() {
                Some(code) if !status.success() => eprintln!(
                    "Warning: `systemctl disable {}` exited with status code: {}",
                    name, code
                ),
                None => eprintln!("Warning: `systemctl disable {}` terminated by signal", name),
                _ => {}
            }

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
