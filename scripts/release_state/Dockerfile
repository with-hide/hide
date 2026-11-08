# One image for both machines of a release-state run: the core and the
# dialed device. The build context is the generator's work folder, which
# holds the release's own programs under `hide/` beside this folder's files.
FROM ubuntu:24.04 AS build
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
COPY claude.c /src/claude.c
RUN gcc -O2 -o /src/claude /src/claude.c

FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    openssh-server openssh-client git python3 ca-certificates procps \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/claude /usr/local/bin/claude
COPY agent.py /opt/agent.py
COPY hide /opt/hide
