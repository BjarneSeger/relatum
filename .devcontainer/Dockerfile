# Debian trixie matches the runtime image (see ../Dockerfile). Any reasonably
# recent Rust base works for the server + CLI workspace.
FROM rust:1-trixie

# System libraries:
#   - build-essential / pkg-config / cmake: native build deps for crates like ring.
#   - libssl-dev: reqwest's default (native) TLS.
#   - postgresql-client / git / curl: developer conveniences.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        cmake \
        libssl-dev \
        postgresql-client \
        git \
        curl \
        ca-certificates \
        sudo \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# Add the components rust-analyzer drives.
RUN rustup component add clippy rustfmt rust-analyzer rust-src

# Run as a non-root user. The official rust image keeps CARGO_HOME / RUSTUP_HOME
# under /usr/local, so hand those to the new user as well.
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID
RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m -s /bin/bash $USERNAME \
    && chown -R $USER_UID:$USER_GID "$CARGO_HOME" "$RUSTUP_HOME" \
    && echo "$USERNAME ALL=(root) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

USER $USERNAME
