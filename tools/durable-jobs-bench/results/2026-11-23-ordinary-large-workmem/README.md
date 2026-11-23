# Ordinary V1 large work_mem freeze

This packet preserves the build-free qualification request and the exact
PostgreSQL configuration overlay. In the planned matched series, ordinary
Mesh, Oban, River, and Apalis all use EXTERNAL payload storage and 16 MiB
`work_mem`. Two ordinary Mesh controls use 4 MiB. Every lane uses the same
64 × 512 KiB workload and shares CPUs 0 and 1 with PostgreSQL 17.11. The
runner and broker binaries match the completed ordinary large series.
[Executable provenance](../2026-11-23-ordinary-trusted-v1-large/README.md)

Qualification checks the effective PostgreSQL settings, input payloads,
ordered independent app commits, terminal rows, TLS, and live connection
caps. The receipt records claim spill blocks, temp-file activity by role,
and sampled PostgreSQL RSS and PSS. Native timing is conditional on the
sealed six-arm qualification and observed removal of the 4 MiB claim
spill at 16 MiB. [Exact request](request.json) · [Freeze manifest](freeze-02/freeze.json)
