# Disclaimer

This project is still in development, not for use in production environment!
I (the author) is not accountable for anything that might go wrong!

# What is singularity-compose-rs ?

`singularity-compose-rs` is a simple CLI tool designed to bring some of the benefits of using `docker-compose` to singularity.

The goal is to:
- define services as singularity instances
- make sure all these instances are running together at startup.

The idea is to have a single file where you define all your singularity-based services, and have singularity-compose-rs update the service files for you.

# Usage

`singularity-compose-rs` is just a unit files builder. It will write all the appropriate unit files in order for your singularity instances to be defined as services. It then allows you to bring them up all at once, or take them down all at once.

You can also specify service groups, and choosing which groups you'd like to build, bring up, or take down.

The CLI has several subcomands:

## Build
