# Todo

## Build command lines

* `cargo watch -x run -w src/`
* `cargo build --release --target=x86_64-unknown-linux-gnu`

## Packaging the server

* [Getting started with Docker images](https://depot.dev/blog/docker-build-image)
* [Create Docker image](https://docs.docker.com/build/building/base-images/)
* [Restarting a Linux process](https://www.baeldung.com/linux/restart-running-process-failure)

## My docker commands

* Build (works): `docker build . -t chimera-md:test`
* Build (cross, not working): `docker build --platform linux/amd64 . -t chimera-md:test`
* Shell in image (won't work with multi-stage image): `docker run -it chimera-md:test bash`
* Run: `docker run -it -p 8080:8080 chimera-md:test`
* Run with override: `docker run -it -p 8080:8080 -e CHIMERA_LOG_LEVEL=TRACE chimera-md:test`
* Run and delete when done: `docker run -it -rm -p 8080:8080 chimera-md:test`
* Tar for sneakernet: `docker save chimera-md:test > chimera_test.tar`
* Load tar on NAS: `sudo docker load -i chimera_test.tar`

Target Linux for docker build

* docker buildx build --platform linux/amd64 -t apps:4.0.0 -f Dockerfile .

## Page design

There are some interesting design elements on the [Tantivy tutorial](https://tantivy-search.github.io/examples/basic_search.html)

* Note the popup menu
* 2 tone background
* Vertical alignment of elements across the two columns

I also admire [this look](https://ryhl.io/blog/actors-with-tokio/). Nice and simple two panel design
