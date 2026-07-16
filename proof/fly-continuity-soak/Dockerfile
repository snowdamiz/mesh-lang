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
    cargo build --locked -q -p meshc --bin meshc \
    && test -x ./target/debug/meshc \
    && install -m 0755 ./target/debug/meshc /tmp/meshc

FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982 AS runtime

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates wget gnupg \
    libssl3 libzstd1 libffi8 libxml2 libz3-4 libedit2 libtinfo6 \
    && wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | apt-key add - \
    && echo "deb http://apt.llvm.org/jammy/ llvm-toolchain-jammy-21 main" \
       > /etc/apt/sources.list.d/llvm.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends libllvm21 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/meshc /usr/local/bin/meshc
COPY proof/fly-continuity-soak/run.sh /usr/local/bin/run-continuity-soak
RUN chmod 0755 /usr/local/bin/meshc /usr/local/bin/run-continuity-soak \
    && /usr/local/bin/meshc --version

ENV RUST_BACKTRACE=1
CMD ["/usr/local/bin/run-continuity-soak"]
