#!/bin/sh

docker volume create flash-fuzz_failures

docker build -t fuzz . 
docker run -v "flash-fuzz_failures:/home/code/run/failures" -it fuzz
