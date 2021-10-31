#!/bin/sh

docker volume create flash-fuzz_failures
docker build -t flash-fuzz_fuzzer .
docker stack deploy --compose-file=docker-compose.yaml fuzzer



