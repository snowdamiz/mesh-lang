"""Replace the failed process-FD witness in the sealed observer."""

from roster import SESSION_COLUMNS


def rewrite_observer(source):
    start = source.index("def ports(")
    end = source.index("def scoped_frame(", start)
    source = source[:start] + source[end:]
    for old, new in (
        ("import re\n", "from collections import Counter\n"),
        ("import backend\n", "import backend\nfrom roster import BOTH, FIRST, ROSTER, connection_roster\n"),
        ("SELECT p.pid, p.usename, p.client_addr, p.client_port, p.state, ssl.ssl",
         "SELECT " + SESSION_COLUMNS),
        ('capture["runner_ports"] = {pid: ports(pid, self.port) for pid in self.runner_pids}',
         'capture["runner_ports"] = connection_roster(self.roster_witness)[1]'),
        ('''    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        self.cluster.runner_pids = [child.pid for child in processes]''',
         '''    def roster(self, expected):
        def ready():
            row = json.loads(self.cluster.sql("mesh", ROSTER))
            counts = Counter(s["usename"] for s in row["sessions"] if s["usename"] in BOTH)
            return row if counts == expected else None

        return ARM.h.wait_for(ready, time.monotonic() + 12, "owned PostgreSQL TLS roster", 0.05)

    def run(self, env, index, log):
        if index == 1:
            self.first_roster = self.roster(FIRST)
            assert self.first_runner.poll() is None
        child = super().run(env, index, log)
        if index == 0:
            self.first_runner = child
        return child

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        assert len(processes) == 2 and processes[0] is self.first_runner
        assert all(child.poll() is None for child in processes)
        witness = dict(first=self.first_roster, both=self.roster(BOTH),
                       runners_by_index=[child.pid for child in processes])
        pinned, _ = connection_roster(witness)
        standard = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        assert {(row["backend"], row["role"], row["tls"]) for row in standard} == {
            (pid, values[0], True) for pid, values in pinned.items()}
        self.cluster.roster_witness = witness
        ARM.h.write_json(self.cluster.evidence / "grant-roster.json", witness)'''),
    ):
        if source.count(old) != 1:
            raise ValueError(f"sealed observer anchor drift: {old[:80]}")
        source = source.replace(old, new)
    return source
