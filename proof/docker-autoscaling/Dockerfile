FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982 AS builder

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    curl wget gnupg git build-essential pkg-config ca-certificates xz-utils \
    libssl-dev libzstd-dev libffi-dev libxml2-dev \
    && rm -rf /var/lib/apt/lists/*

# apt.llvm.org drops the odd request. Piping `wget -q` into apt-key turned
# that into "no valid OpenPGP data"; retry, and fail on the download itself.
RUN curl --fail --silent --show-error --location --retry 5 --retry-all-errors \
        https://apt.llvm.org/llvm-snapshot.gpg.key -o /tmp/llvm-snapshot.gpg.key \
    && apt-key add /tmp/llvm-snapshot.gpg.key \
    && echo "deb http://apt.llvm.org/jammy/ llvm-toolchain-jammy-21 main" \
       > /etc/apt/sources.list.d/llvm.list \
    && apt-get -o Acquire::Retries=5 update \
    && apt-get -o Acquire::Retries=5 install -y llvm-21-dev libpolly-21-dev clang-21 \
    && rm -rf /var/lib/apt/lists/* /tmp/llvm-snapshot.gpg.key

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y --default-toolchain 1.92.0
ENV PATH="/root/.cargo/bin:${PATH}"
ENV LLVM_SYS_211_PREFIX=/usr/lib/llvm-21
ENV LLVM_LINK_LLVM_DYLIB=1
ENV CARGO_TARGET_DIR=/app/target

WORKDIR /app
COPY . .

# MESH_COVERAGE=1 (`meshc proof docker-autoscaling` with
# MESH_PROOF_COVERAGE_DIR set) instruments the runtime the proof program links
# and the capacity driver, with counters kept in the profile file as they
# change (continuous mode): the proof kills nodes, which then write nothing at
# exit. A static library leaves out the profiler runtime, so the linker is
# given it the way rustc gives it for binaries: last, and asked for (`-u`).
# Linked any earlier, its default counter bias wins over the runtime's.
ARG MESH_COVERAGE=0
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --locked --release -q -p mesh-rt -p meshc \
    && test -x ./target/release/mesh-capacity-driver \
    && install -m 0755 ./target/release/mesh-capacity-driver /tmp/mesh-capacity-driver \
    && mkdir -p /tmp/coverage \
    && if [ "$MESH_COVERAGE" = 1 ]; then \
         RUSTFLAGS="-C instrument-coverage -C llvm-args=-runtime-counter-relocation" \
           CARGO_TARGET_DIR=/app/target/coverage cargo build --locked --release -q -p mesh-rt \
         && profiler="$(ls "$(rustc --print sysroot)"/lib/rustlib/*/lib/libprofiler_builtins-*.rlib)" \
         && cp /app/target/coverage/release/libmesh_rt.a /tmp/coverage/libmesh_rt.a \
         && install -m 0755 /app/target/coverage/release/mesh-capacity-driver /tmp/mesh-capacity-driver \
         && cp /tmp/mesh-capacity-driver /tmp/coverage/mesh-capacity-driver \
         && export MESH_RT_LIB_PATH=/tmp/coverage/libmesh_rt.a \
         && mkdir -p /tmp/coverage-bin \
         && printf '#!/bin/sh\nexec /usr/bin/cc "$@" -Wl,-u,__llvm_profile_runtime %s\n' "$profiler" > /tmp/coverage-bin/cc \
         && chmod 0755 /tmp/coverage-bin/cc \
         && export PATH="/tmp/coverage-bin:$PATH"; \
       fi \
    && ./target/release/meshc build examples/todo-postgres \
       --opt-level 2 --output /tmp/mesh-autoscaling-proof --no-color \
    && test -x /tmp/mesh-autoscaling-proof

# The instrumented runtime and driver, exported to the host for reading the
# containers' profiles (built only for coverage).
FROM scratch AS coverage-objects
COPY --from=builder /tmp/coverage/ /

FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90 AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home mesh

COPY --from=builder /tmp/mesh-autoscaling-proof /usr/local/bin/mesh-autoscaling-proof
RUN chmod 0755 /usr/local/bin/mesh-autoscaling-proof

USER mesh:mesh
EXPOSE 8080 4370
ENTRYPOINT ["/usr/local/bin/mesh-autoscaling-proof"]

FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90 AS driver

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates docker.io \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/mesh-capacity-driver /usr/local/bin/mesh-capacity-driver
RUN chmod 0755 /usr/local/bin/mesh-capacity-driver

EXPOSE 7443
ENTRYPOINT ["/usr/local/bin/mesh-capacity-driver"]
