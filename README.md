# singularity-compose-rs

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

`singularity-compose-rs` is a simple CLI tool designed to bring some of the benefits of using `docker-compose` to Singularity.

The goal is to:
- define services as singularity instances
- make sure all these instances are running together at startup.

The idea is to have a single file where you define all your singularity-based services, and have singularity-compose-rs update the service files for you.

## Supported Service Keywords

| Keyword         | Type            | Description                                                                                                                                    |
| --------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `service_name`  | string (req.)   | Unique identifier for the service. Must not contain line breaks.                                                                               |
| `description`   | string (req.)   | Human-readable description of the service. Must not contain line breaks.                                                                       |
| `image`         | string (req.)   | Absolute path to the Singularity image (`.sif`) file.                                                                                          |
| `user`          | string (opt.)   | The user to run the Singularity instance as. Defaults to `root`.                                                                               |
| `group`         | string (opt.)   | The group to run the Singularity instance as. Defaults to `root`.                                                                              |
| `volumes`       | list of strings | Bind mounts formatted as `<host_path>:<container_path>[:ro]`.                                                                                  |
| `pidfile`       | string (opt.)   | Path to the PID file. Defaults to `/run/<service_name>.pid`.                                                                                   |
| `restart`       | string (opt.)   | Restart condition. Must be one of: `no`, `always`, `on-success`, `on-failure`, `on-abnormal`, `on-abort`, `on-watchdog`. Defaults to `always`. |
| `after`         | string (opt.)   | systemd `After=` dependency (e.g. `network-online.target`). Defaults to `network-online.target`.                                               |
| `requires`      | string (opt.)   | systemd `Requires=` dependency (e.g. `NetworkManager.service`). Defaults to `network-online.target`.                                           |
| `service_group` | string (opt.)   | Dot-separated group hierarchy used for filtering with the `--groups` flag (e.g. `web.essential`).                                              |

### Service Groups

Service groups support a hierarchy expressed with `.`. Requesting group `g` matches services whose group is `g` or whose group starts with `g.`. For example, requesting `web` matches both `web.essential` and `web.optional`.

## Install

You can install the project with cargo:

```bash
cargo install --path .
```

## Usage

`singularity-compose-rs` is just a unit files builder. It will write all the appropriate unit files in order for your singularity instances to be defined as services. It then allows you to bring them up all at once, or take them down all at once.

The compose file is read from `/etc/singularity-compose-rs/compose.yaml` by default.

You can also specify service groups, and choosing which groups you'd like to build, bring up, or take down.

### Global Flags

| Flag                | Short | Description                                                                                                                                     |
| ------------------- | ----- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `--groups <GROUPS>` | `-g`  | Comma-separated list of groups to select. Supports dot-separated hierarchy (e.g. `web.essential`). When omitted, **all** services are selected. |
| `--file <FILE>`     | `-f`  | Path to the compose file (on `add` only).                                                                                                       |
| `--dry-run`         | `-n`  | Print what would be done without making any changes. Not available for `add` and `remove`.                                                      |

### Build

Builds all the unit files from the definition file.

```bash
singularity-compose-rs build [OPTIONS]
```

Builds the unit files for all services by default. Use `--groups` to restrict to a specific set of services.
This calls `systemctl daemon-reload` so the new service files are accounted for by systemd, but it does **not** bring them up.
It is up to the user to bring them up using the `up` command.

### Up

Starts and enables the selected services.

```bash
singularity-compose-rs up [OPTIONS]
```

### Down

Stops the selected services.

```bash
singularity-compose-rs down [OPTIONS]
```

### List

Lists the selected services in a tree format.

```bash
singularity-compose-rs list [OPTIONS]
```

Note: this command is the only one that does **not** require root privileges.

### Add

Merges a new compose file into the existing one and (re)-builds the affected unit files.

```bash
singularity-compose-rs add --file <FILE>
```

This command only stops, disables, and overwrites unit files for services that are re-defined in the input file. Newly added services are built and unit files are written.

Note: this requires root privileges. **There is no dry-run mode for this command; use with caution.**

### Remove

Removes one or more services from the compose file, stops and disables their unit files, and removes the unit files.

```bash
singularity-compose-rs remove <SERVICE_NAME>...
```

Note: this requires root privileges.

## Example Compose File

```yaml
services:
  - service_name: nginxd
    description: Nginx reverse proxy
    user: root
    group: root
    volumes:
      - /root/nginx/nginx.conf:/opt/nginx.conf
      - /var/log
      - /var/cache
    pidfile: /run/nginxd.pid
    image: /root/nginx/nginx.sif
    restart: always
    after: NetworkManager.service
    requires: NetworkManager.service
    service_group: web.essential

  - service_name: gitead
    description: Gitea, the web application for git
    user: charles
    group: charles
    volumes:
      - /home/charles/gitea-cluster-install/data-gitea:/data/gitea
    pidfile: /home/charles/gitea.pid
    image: /data/singularity_images/gitea-1.26.4.sif
    after: NetworkManager.service
    requires: NetworkManager.service scompose-nginxd.service
    service_group: web.optional
```

## How It Works

For each service defined in the compose file, singularity-compose-rs generates a systemd unit file at `/etc/systemd/system/scompose-<service_name>.service`. The generated unit file uses a forking service type and starts the Singularity instance using:

```bash
singularity instance start --pid-file <pidfile> <binds> <image> <service_name>
```

When `build` is run, the tool validates the compose file, checks that the Singularity images exist and are absolute paths, and writes the unit files. After building, `systemctl daemon-reload` is run automatically.

`up` starts and enables the generated unit files. `down` stops them.

## License

MIT
