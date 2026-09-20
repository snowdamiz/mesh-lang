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

ARG DATA_APP
ARG CONTROLLER_APP
ARG WORKER_IMAGE
ARG CLUSTER_ID
ARG VOTERS
ARG REGION
ARG RUN_ID

WORKDIR /app
COPY . .

RUN test -n "${DATA_APP}" \
    && test -n "${CONTROLLER_APP}" \
    && test -n "${WORKER_IMAGE}" \
    && test -n "${CLUSTER_ID}" \
    && test -n "${VOTERS}" \
    && test -n "${REGION}" \
    && test -n "${RUN_ID}" \
    && cp proof/fly-autoscaling/mesh.toml examples/todo-postgres/mesh.toml \
    && sed -i \
       -e "s#@@DATA_APP@@#${DATA_APP}#g" \
       -e "s#@@CONTROLLER_APP@@#${CONTROLLER_APP}#g" \
       -e "s#@@WORKER_IMAGE@@#${WORKER_IMAGE}#g" \
       -e "s#@@CLUSTER_ID@@#${CLUSTER_ID}#g" \
       -e "s#@@VOTERS@@#${VOTERS}#g" \
       -e "s#@@REGION@@#${REGION}#g" \
       -e "s#@@RUN_ID@@#${RUN_ID}#g" \
       examples/todo-postgres/mesh.toml \
    && ! grep -q '@@' examples/todo-postgres/mesh.toml

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --locked -q -p meshc --bin meshc \
    && ./target/debug/meshc build examples/todo-postgres \
       --output /tmp/mesh-fly-autoscaling-app --no-color \
    && install -m 0755 ./target/debug/meshc /tmp/meshc \
    && test -x /tmp/mesh-fly-autoscaling-app

FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90 AS app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home mesh

COPY --from=builder /tmp/mesh-fly-autoscaling-app /usr/local/bin/mesh-fly-autoscaling-app
RUN chmod 0755 /usr/local/bin/mesh-fly-autoscaling-app

USER mesh:mesh
EXPOSE 8080 4370
ENTRYPOINT ["/usr/local/bin/mesh-fly-autoscaling-app"]

FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982 AS runner

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates curl jq postgresql-client wget gnupg \
       libssl3 libzstd1 libffi8 libxml2 libz3-4 libedit2 libtinfo6 \
    && wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key | apt-key add - \
    && echo "deb http://apt.llvm.org/jammy/ llvm-toolchain-jammy-21 main" \
       > /etc/apt/sources.list.d/llvm.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends libllvm21 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/meshc /usr/local/bin/meshc
COPY proof/fly-autoscaling/run.sh /usr/local/bin/run-fly-autoscaling-proof
COPY proof/docker-autoscaling/init.sql /usr/local/share/mesh-proof/init.sql
RUN chmod 0755 /usr/local/bin/meshc /usr/local/bin/run-fly-autoscaling-proof

ENV RUST_BACKTRACE=1
ENTRYPOINT ["/usr/local/bin/run-fly-autoscaling-proof"]
