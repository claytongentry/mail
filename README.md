# mail

Small IMAPrev1-style Rust server.

## Development

This repo uses Apple's `container` CLI for a local Rust 1.96 environment. Start
the container service once before using these commands:

```sh
container system start
```

Common commands:

```sh
just build
just test
just start
```

`just start` builds the image, runs the server in a container, and publishes it
on `127.0.0.1:1143`. Override the local port or JWT secret with environment
variables:

```sh
MAIL_PORT=2143 JWT_SECRET=local-secret just start
```
