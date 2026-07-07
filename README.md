# singularity-compose-rs

`singularity-compose-rs` is a simple CLI tool designed to bring some of the benefits of using `docker-compose` to Singularity.

The goal is to:
- define singularity instances as services
- make sure all these instances are running together at startup

The idea is to have a single file where you define all your singularity-based services, and have singularity-compose-rs update the service files for you.
To help even further with managing arbitrarily complex service setups, this tool also allows one to assign hierarchical groups to each individual service definition.

## Requirements

This software only works on Linux, as it's a tool designed to work with `singularity` and `systemd`.
So you need a Linux-based OS, with `systemd` installed (which should be the default for any recent distribution).

You also need to have singularity installed, at `/usr/bin/singularity` for now. This might change in the future if I feel like adding `singularity_path` as an optional field. Please open an issue if you really need it.
If `/usr/bin/singularity` doesn't exist (you've installed `singularity` at another location, or you have `apptainer` and no alias to `singularity`), you may want to run `ln -s <ACTUAL BINARY PATH> /usr/bin/singularity`.


## Service definition keywords

| Keyword         | Type                           | Description                                                                                                                                                                           |
| --------------- | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `service_name`  | string (required)              | Unique identifier for the service. Must not contain line breaks.                                                                                                                      |
| `image`         | string (required)              | Absolute path to the Singularity image (`.sif`) file.                                                                                                                                 |
| `description`   | string (optional)              | Human-readable description of the service. Must not contain line breaks.                                                                                                              |
| `user`          | string (optional)              | The user to run the Singularity instance as. Defaults to `root`.                                                                                                                      |
| `group`         | string (optional)              | The group to run the Singularity instance as. Defaults to `root`.                                                                                                                     |
| `volumes`       | list of strings (can be empty) | Bind mounts formatted as `<host_path>[:<container_path>[:ro]]`.                                                                                                                       |
| `pidfile`       | string (optional)              | Path to the PID file. Defaults to `/run/<service_name>.pid`. Make sure the specified user/group can write it.                                                                         |
| `restart`       | string (optional)              | Restart condition. Must be one of: `no`, `always`, `on-success`, `on-failure`, `on-abnormal`, `on-abort`, `on-watchdog`. Defaults to `always`.                                        |
| `after`         | string (optional)              | systemd `After=` dependency (e.g. `network-online.target`). Defaults to `network-online.target`.                                                                                      |
| `requires`      | string (optional)              | systemd `Requires=` dependency (e.g. `NetworkManager.service`). Defaults to `network-online.target`.                                                                                  |
| `service_group` | string (optional)              | Dot-separated group hierarchy used for filtering with the `--groups` flag (e.g. `web.essential` will make this service both part of the `web` and the `essential` subgroup of `web`). |

Please note that if referring to another service managed by `singularity-compose-rs`, you have to prefix it with `scompose-`. This applies to fields `requires` and `after`.
See the example [below](#example-compose-file).

### Service Groups

Service groups support a hierarchy expressed with `.`.
There are only used to refer to service definitions within the master `compose.yaml` file (in `/etc/singularity-compose-rs`) and in this CLI. They are completely ignored by systemd.

## Install

### Using cargo

You can install the project with cargo:

```bash
git clone git@github.com:SteampunkIslande/singularity-compose-rs.git
cd singularity-compose-rs
cargo install --path .
```

### By copying the binary

```bash
git clone git@github.com:SteampunkIslande/singularity-compose-rs.git
cd singularity-compose-rs
cargo build --release
# Or copy it to wherever you want in your path
sudo cp target/x86_64-unknown-linux-musl/release/singularity-compose-rs /usr/bin
# You can even define an alias
sudo ln -s /usr/bin/singularity-compose-rs /usr/bin/scompose
```

The resulting binary is 100% standalone, you can just copy it and it will work on any linux computer with `Linux kernel >=2.6.39` (so basically any linux distribution will do).


## Usage

`singularity-compose-rs` is just a unit files builder. It will write all the appropriate unit files in order for your singularity instances to be defined as services. It then allows you to bring them up all at once, take them down all at once, or let you choose which groups you'd like to start/stop.

The compose file is read from `/etc/singularity-compose-rs/compose.yaml` and this cannot be changed.

You can also specify service groups, and choosing which groups you'd like to build, bring up, or take down.

General usage
```
Usage: scompose <COMMAND>

Commands:
  build   (Re)-builds all the unit files
  up      Brings all specified services up
  down    Shuts down all the services that are defined in the singularity-compose.yaml file (or the file specified with --file)
  list    
  add     Merge a compose file into the existing one and (re)-builds
  remove  Remove one or more services from the compose file and stop/disable their unit files
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

### Build

Builds the unit files for all services by default. Use `--groups` to restrict to a specific set of services.

```
(Re)-builds all the unit files

Usage: scompose build [OPTIONS]

Options:
  -n, --dry-run
          Will not write any unit files, only print
          
          This will print every file name, and every file content of all the unit files that would be written without this flag.

  -g, --groups [<GROUPS>...]
          Groups you want to (re)-build (comma-separated)
          
          Note that you can express a group hierarchy with `.`. If omitted, this will build all services defined in `/etc/singularity-compose-rs/compose.yaml`

  -h, --help
          Print help (see a summary with '-h')
```

This calls `systemctl daemon-reload` so the new service files are accounted for by systemd, but it does **not** bring them up.
It is up to the user to bring them up using the `up` command.

### Up

Starts all the services by default. Use `--groups` to restrict to a specific set of services.

```
Brings all specified services up

Usage: scompose up [OPTIONS]

Options:
  -n, --dry-run
          Will not start any service, only print
          
          This will print the systemctl command that would be run without this flag.

  -g, --groups [<GROUPS>...]
          Groups you want to start
          
          Note that you can express a group hierarchy with `.`. If omitted, this will build all services defined in `/etc/singularity-compose-rs/compose.yaml`

  -h, --help
          Print help (see a summary with '-h')
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
    user: USERNAME
    group: USERNAME
    volumes:
      - /home/USERNAME/gitea-install/data-gitea:/data/gitea
    pidfile: /home/USERNAME/gitea.pid
    image: /data/singularity_images/gitea-1.26.4.sif
    after: NetworkManager.service
    requires: NetworkManager.service scompose-nginxd.service
    service_group: web.optional
```

This very basic example lets you compose a simple webapp setup with [`nginx`](https://nginx.org/) and [`gitea`](https://about.gitea.com/) to have your own, self-hosted git webapp (given that you properly configured `nginx.conf` and your dedicated `gitea-data` folder).

## How It Works

For each service defined in the compose file, `singularity-compose-rs` generates a systemd unit file at `/etc/systemd/system/scompose-<service_name>.service`. The generated unit file uses a forking service type and starts the Singularity instance using:

```bash
singularity instance start --pid-file <pidfile> <binds> <image> <service_name>
```

When `build` is run, the tool validates the compose file, checks that the Singularity images exist and are absolute paths, and writes the unit files. After building, `systemctl daemon-reload` is run automatically.

`up` starts and enables the generated unit files. `down` stops them.

## License

MIT
