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
just smoke
```

`just start` builds the image, runs the server in a container, and publishes it
on `127.0.0.1:1143`. By default it uses the in-memory fixture mail store.
Override the local port or JWT secret with environment variables:

```sh
MAIL_PORT=2143 JWT_SECRET=local-secret just start
```

In another terminal, run the scripted imaptest compliance smoke suite against the
running server:

```sh
JWT_SECRET=local-secret just smoke
```

`just smoke` runs the Dovecot `imaptest` binary inside the running
`mail-dev` container. The target generates an XOAUTH2 initial response from
the running container's `JWT_SECRET` and runs the scripts copied into
`/app/tests/imaptest`. Use `IMAPTEST_ARGS` for additional imaptest flags.

By default, `just smoke` reports all scripted failures. To stop at the first
failure while debugging one issue, run:

```sh
IMAPTEST_ARGS=error_quit JWT_SECRET=local-secret just smoke
```

To use the SQLite mail store, opt in with `MAIL_STORE=sqlite`:

```sh
MAIL_STORE=sqlite just start
```

The default database path is `/data/mail.sqlite3`. `just start` mounts the
Apple `container` named volume `mail-data` at `/data`, so the SQLite database
persists across container restarts. Create the volume explicitly if you want to
see setup failures before starting the server:

```sh
just volume
```

Override the volume or database path when needed:

```sh
MAIL_VOLUME=my-mail-data MAIL_DB_PATH=/data/dev.sqlite3 MAIL_STORE=sqlite just start
```

To reset the local SQLite state, stop the server and delete the named volume:

```sh
just stop
container volume delete mail-data
```
