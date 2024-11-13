#!/bin/zsh

version=$(cargo metadata --format-version 1  | jq -r '.packages[]  | select(.name | test("chimera-md")) | .version');

echo "Building docker image for version: $version";
docker build --platform linux/amd64 . -t acbarrentine/chimera-md:$version;
docker push acbarrentine/chimera-md:$version;
# docker push acbarrentine/chimera-md:latest;

