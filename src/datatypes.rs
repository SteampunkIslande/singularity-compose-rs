use serde::Deserialize;

#[derive(Debug, Deserialize)]
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
}

#[derive(Debug, Deserialize)]
pub struct Document {
    pub services: Vec<Service>,
}
