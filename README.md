# What is singularity-compose-rs ?

`singularity-compose-rs` is a simple CLI tool designed to bring some of the benefits of using `docker-compose` to singularity.

The goal is to make sure all your services are running together at startup.

# Usage

`singularity-compose-rs` is just a unit files builder for systemd. It will write all the appropriate unit files in order for your singularity instances to be defined as services. It then allows you to make them up all at once, or down all at once.

The CLI has several subcomands:

## Build
