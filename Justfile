image := "mail-dev:latest"
test_image := "mail-dev:test"
container_name := "mail-dev"
port := env_var_or_default("MAIL_PORT", "1143")
jwt_secret := env_var_or_default("JWT_SECRET", "dev-secret")

default:
    @just --list

build:
    container build -f Containerfile -t {{image}} .

test:
    container build -f Containerfile --target test -t {{test_image}} .

start: build
    container run --rm --name {{container_name}} -p 127.0.0.1:{{port}}:1143 -e JWT_SECRET='{{jwt_secret}}' -e IMAP_BIND_ADDR=0.0.0.0:1143 {{image}}

shell: build
    container run --rm -it --entrypoint /bin/bash -e JWT_SECRET='{{jwt_secret}}' {{image}}

stop:
    -container stop {{container_name}}
