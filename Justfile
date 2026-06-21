image := "mail-dev:latest"
test_image := "mail-dev:test"
container_name := "mail-dev"
port := env_var_or_default("MAIL_PORT", "1143")
jwt_secret := env_var_or_default("JWT_SECRET", "dev-secret")
mail_store := env_var_or_default("MAIL_STORE", "fixture")
mail_db_path := env_var_or_default("MAIL_DB_PATH", "/data/mail.sqlite3")
mail_volume := env_var_or_default("MAIL_VOLUME", "mail-data")
imaptest_bin := env_var_or_default("IMAPTEST_BIN", "imaptest")
imaptest_user := env_var_or_default("IMAPTEST_USER", "test@example.com")
imaptest_tests := env_var_or_default("IMAPTEST_TESTS", "/app/tests/imaptest")
imaptest_args := env_var_or_default("IMAPTEST_ARGS", "")

default:
    @just --list

build:
    container build -f Containerfile -t {{image}} .

test:
    container build -f Containerfile --target base -t {{test_image}} .
    container run --rm {{test_image}} cargo test --locked

smoke:
    @container inspect {{container_name}} >/dev/null || { echo "{{container_name}} is not running. Start it with 'just start' before running smoke."; exit 1; }
    @container exec {{container_name}} /bin/sh -c 'command -v {{imaptest_bin}} >/dev/null' || { echo "{{imaptest_bin}} is not installed in {{container_name}}. Restart it with 'just stop' then 'just start' to pick up the rebuilt dev image."; exit 127; }
    @XOAUTH2_RESPONSE="$(container exec {{container_name}} /bin/sh -c 'IMAPTEST_USER='"'"'{{imaptest_user}}'"'"' sh /app/bin/imaptest-xoauth2')" ; \
      container exec {{container_name}} {{imaptest_bin}} host=127.0.0.1 port=1143 user='{{imaptest_user}}' pass="$XOAUTH2_RESPONSE" test='{{imaptest_tests}}' no_pipelining {{imaptest_args}}

start: build
    container run --rm --name {{container_name}} -p 127.0.0.1:{{port}}:1143 -v {{mail_volume}}:/data -e JWT_SECRET='{{jwt_secret}}' -e IMAP_BIND_ADDR=0.0.0.0:1143 -e MAIL_STORE='{{mail_store}}' -e MAIL_DB_PATH='{{mail_db_path}}' {{image}}

shell: build
    container run --rm -it --entrypoint /bin/bash -v {{mail_volume}}:/data -e JWT_SECRET='{{jwt_secret}}' -e MAIL_STORE='{{mail_store}}' -e MAIL_DB_PATH='{{mail_db_path}}' {{image}}

volume:
    container volume create {{mail_volume}}

stop:
    -container stop {{container_name}}
