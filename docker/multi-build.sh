version=$(cargo metadata --format-version 1  | jq -r '.packages[]  | select(.name | test("chimera-md")) | .version');

echo "Building multi-platform docker image for version: $version";
docker buildx build --platform linux/amd64,linux/arm64 . --push -t acbarrentine/chimera-md:$version -t acbarrentine/chimera-md:latest;
