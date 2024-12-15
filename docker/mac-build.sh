#!/bin/zsh

# version=$(cargo metadata --format-version 1  | jq -r '.packages[]  | select(.name | test("chimera-md")) | .version');
version=latest

echo "Building docker image for Mac version: $version";
# cargo build --release
docker build -t acbarrentine/chimera-md-mac:$version -f Dockerfile .;
# docker build -t acbarrentine/chimera-md-mac:$version -f native-dockerfile .;
