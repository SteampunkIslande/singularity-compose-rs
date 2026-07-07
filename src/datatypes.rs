use std::{collections::HashSet, path::Path};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::error::SingularityComposeError;

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Service {
    pub service_name: String,
    pub description: String,
    pub user: Option<String>,
    pub group: Option<String>,
    pub volumes: Vec<String>,
    pub pidfile: Option<String>,
    pub image: String,
    pub restart: Option<String>,
    pub after: Option<String>,
    pub requires: Option<String>,
    pub service_group: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Document {
    pub services: Vec<Service>,
}

pub struct MergeResult {
    pub unchanged: Vec<Service>,
    pub added: Vec<Service>,
    pub overwritten: Vec<Service>,
}

impl Document {
    /// Returns the services whose `service_group` matches any of the requested groups.
    ///
    /// Groups support a hierarchy expressed with `.`. A requested group `g` matches a
    /// service group `s` when `s == g` or `s` is a descendant of `g` (i.e. `s` starts with
    /// `g.`). For example, requesting `web` matches both `web.essential` and `web.optional`,
    /// while requesting `web.optional` matches only that group.
    ///
    /// When `groups` is empty, all services are returned (default behaviour).
    pub fn services_for_groups(&self, groups: &[String]) -> Vec<Service> {
        if groups.is_empty() {
            return self.services.to_vec();
        }
        self.services
            .iter()
            .filter(|service| {
                service.service_group.as_ref().is_some_and(|service_group| {
                    groups.iter().any(|requested| {
                        *requested == *service_group
                            || service_group.starts_with(&format!("{requested}."))
                    })
                })
            })
            .cloned()
            .collect()
    }

    pub fn try_from_file_path<T: AsRef<Path>>(file_path: T) -> anyhow::Result<Self> {
        let doc: Document = yaml_serde::from_reader(
            std::fs::File::open(file_path.as_ref())
                .context(format!("Cannot open `{}`", file_path.as_ref().display()))?,
        )?;
        let mut all_service_names: HashSet<&str> = HashSet::new();
        for service in doc.services.iter() {
            if all_service_names.contains(service.service_name.as_str()) {
                bail!(SingularityComposeError::DuplicateService(
                    service.service_name.clone()
                ));
            }
            all_service_names.insert(service.service_name.as_str());
            if service.service_name.as_str().contains("\n") {
                bail!(SingularityComposeError::InvalidField(
                    "Service name cannot contain line breaks".to_string()
                ));
            }
            if service.description.as_str().contains("\n") {
                bail!(SingularityComposeError::InvalidField(
                    "Description cannot contain line breaks".to_string()
                ));
            }
            if service
                .user
                .as_ref()
                .is_some_and(|user| user.contains("\n"))
            {
                bail!(SingularityComposeError::InvalidField(
                    "User name cannot contain line breaks".to_string()
                ));
            }
            if service
                .group
                .as_ref()
                .is_some_and(|group| group.contains("\n"))
            {
                bail!(SingularityComposeError::InvalidField(
                    "Group name cannot contain line breaks".to_string()
                ));
            }
            for volume in service.volumes.iter() {
                if volume.contains("\n") {
                    bail!(SingularityComposeError::InvalidField(
                        "Volumes cannot contain line breaks".to_string()
                    ));
                }
            }
            if service.pidfile.as_ref().is_some_and(|p| p.contains("\n")) {
                bail!(SingularityComposeError::InvalidField(
                    "PIDFile cannot contain line breaks".to_string()
                ));
            }
            if service.image.contains("\n") {
                bail!(SingularityComposeError::InvalidField(
                    "Singularity image file cannot contain line breaks".to_string()
                ));
            }
            if let Some(restart) = service.restart.as_ref()
                && ![
                    "no",
                    "always",
                    "on-success",
                    "on-failure",
                    "on-abnormal",
                    "on-abort",
                    "on-watchdog",
                ]
                .contains(&restart.as_str())
            {
                bail!(SingularityComposeError::InvalidField(format!(
                    "If you specify a restart condition, it should be one of: `no`, `always`,`on-success`,`on-failure`,`on-abnormal`,`on-abort`, or `on-watchdog`; found `{}`",
                    restart
                )));
            }
            if service.after.as_ref().is_some_and(|p| p.contains("\n")) {
                bail!(SingularityComposeError::InvalidField(
                    "After dependencies cannot contain line breaks".to_string()
                ));
            }
            if service.requires.as_ref().is_some_and(|p| p.contains("\n")) {
                bail!(SingularityComposeError::InvalidField(
                    "Requires dependencies cannot contain line breaks".to_string()
                ));
            }
            if service
                .service_group
                .as_ref()
                .is_some_and(|p| p.contains("\n"))
            {
                bail!(SingularityComposeError::InvalidField(
                    "Service group cannot contain line breaks".to_string()
                ));
            }
        }
        Ok(doc)
    }

    /// Remove services whose `service_name` is in `names`.
    ///
    /// The removed services are returned so the caller can act on them
    /// (e.g. stop/disable their unit files).
    pub fn remove_services(&mut self, names: &[String]) -> Vec<Service> {
        let mut removed = Vec::new();
        let mut remaining = Vec::new();
        for service in std::mem::take(&mut self.services) {
            if names.contains(&service.service_name) {
                removed.push(service);
            } else {
                remaining.push(service);
            }
        }
        self.services = remaining;
        removed
    }

    /// Merge another document into this one.
    ///
    /// For each service in `other` that already exists in `self`, the existing service is
    /// overwritten and its name is added to the returned list. For new services, they are
    /// simply appended. After merging, the caller is responsible for ensuring there are no
    /// duplicate service names in the resulting document.
    pub fn merge_document(self, other: Document) -> MergeResult {
        let mut unchanged = self.services;
        let mut overwritten = Vec::new();
        let mut added = Vec::new();

        for service in other.services.into_iter() {
            if let Some(pos) = unchanged
                .iter_mut()
                .position(|s| s.service_name == service.service_name)
            {
                if unchanged[pos] != service {
                    // Add to overwritten, remove from unchanged
                    overwritten.push(service);
                    unchanged.remove(pos);
                }
            } else {
                // This is a new service name
                added.push(service);
            }
        }
        MergeResult {
            unchanged,
            added,
            overwritten,
        }
    }
}

impl From<MergeResult> for Document {
    fn from(value: MergeResult) -> Self {
        Document {
            services: [value.unchanged, value.added, value.overwritten].concat(),
        }
    }
}
