# Toolchain for benchmarks/instructions.py where valgrind is unavailable
# (macOS). Both builds being compared run with this one compiler, so the pin
# only matters for comparing counts across sittings. See benchmarks/README.md.
FROM rust:1.99-slim-trixie
RUN apt-get update \
    && apt-get install -y --no-install-recommends valgrind python3 git \
    && rm -rf /var/lib/apt/lists/*
# The tree is mounted from the host and owned by another user, which git
# refuses to touch; `--base` needs it to add the base worktree.
RUN git config --system --add safe.directory '*'
