image := "mail-dev:latest"
test_image := "mail-dev:test"
container_name := "mail-dev"
port := env_var_or_default("MAIL_PORT", "1143")
jwt_secret := env_var_or_default("JWT_SECRET", "dev-secret")
mail_store := env_var_or_default("MAIL_STORE", "fixture")
mail_db_path := env_var_or_default("MAIL_DB_PATH", "/data/mail.sqlite3")
mail_volume := env_var_or_default("MAIL_VOLUME", "mail-data")

default:
    @just --list

build:
    container build -f Containerfile -t {{image}} .

test:
    container build -f Containerfile --target test -t {{test_image}} .

start: build
    container run --rm --name {{container_name}} -p 127.0.0.1:{{port}}:1143 -v {{mail_volume}}:/data -e JWT_SECRET='{{jwt_secret}}' -e IMAP_BIND_ADDR=0.0.0.0:1143 -e MAIL_STORE='{{mail_store}}' -e MAIL_DB_PATH='{{mail_db_path}}' {{image}}

shell: build
    container run --rm -it --entrypoint /bin/bash -v {{mail_volume}}:/data -e JWT_SECRET='{{jwt_secret}}' -e MAIL_STORE='{{mail_store}}' -e MAIL_DB_PATH='{{mail_db_path}}' {{image}}

volume:
    container volume create {{mail_volume}}

stop:
    -container stop {{container_name}}
