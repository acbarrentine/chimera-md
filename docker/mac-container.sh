docker container rm -f chimera-mac
docker run --hostname=515e12074420 --env=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin --volume=/Users/acbarrentine/Source/dismal.ink/chimera.toml:/data/chimera.toml --volume=/Users/acbarrentine/Source/dismal.ink/image-sizes.toml:/data/image-sizes.toml --volume=/Users/acbarrentine/Source/dismal.ink/www:/data/www --volume=/Users/acbarrentine/Source/dismal.ink/home:/data/home --volume=/Users/acbarrentine/Source/dismal.ink/log:/data/log --volume=/Users/acbarrentine/Source/dismal.ink/search:/data/search --volume=/Users/acbarrentine/Source/dismal.ink/templates:/data/template --volume=/Users/acbarrentine/Paintings:/data/home/media/Paintings:ro --volume=/Users/acbarrentine/Paintings-archived:/data/home/media/Archive:ro --workdir=/ --restart=no --name chimera-mac --runtime=runc -p 8080:8080 -d acbarrentine/chimera-md-mac:latest
