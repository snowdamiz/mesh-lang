FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982 AS builder

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    curl wget gnupg git build-essential pkg-config ca-certificates xz-utils \
    libssl-dev libzstd-dev libffi-dev libxml2-dev \
    && rm -rf /var/lib/apt/lists/*

RUN wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | apt-key add - \
    && echo "deb http://apt.llvm.org/jammy/ llvm-toolchain-jammy-21 main" \
       > /etc/apt/sources.list.d/llvm.list \
    && apt-get update \
    && apt-get install -y llvm-21-dev libpolly-21-dev clang-21 \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y --default-toolchain 1.92.0
ENV PATH="/root/.cargo/bin:${PATH}"
ENV LLVM_SYS_211_PREFIX=/usr/lib/llvm-21
ENV LLVM_LINK_LLVM_DYLIB=1
ENV CARGO_TARGET_DIR=/app/target

WORKDIR /app
COPY . .

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --locked -q -p mesh-rt -p meshc \
    && test -x ./target/debug/mesh-capacity-driver \
    && install -m 0755 ./target/debug/mesh-capacity-driver /tmp/mesh-capacity-driver \
    && ./target/debug/meshc build examples/todo-postgres \
       --output /tmp/mesh-autoscaling-proof --no-color \
    && test -x /tmp/mesh-autoscaling-proof

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
