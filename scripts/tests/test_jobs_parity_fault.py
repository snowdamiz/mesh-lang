import socket
import struct
import unittest
from contextlib import contextmanager

from scripts.lib.jobs_parity_fault import SettlementProxy


def message(kind, body=b""):
    return kind + struct.pack("!I", len(body) + 4) + body


def startup():
    body = b"\0\x03\0\0user\0parity_subject\0\0"
    return struct.pack("!I", len(body) + 4) + body


OBAN_SQL = b'UPDATE "probe"."oban_jobs" SET "completed_at" = $1'
MESH_SQL = b"SELECT coalesce(mesh_jobs.settle_state($1, $2, $3, $4, false), 'stale')"
QUERY = message(b"Q", OBAN_SQL + b"\0")
IDLE = message(b"Z", b"I")
SYNC = message(b"S")
COMPLETE = message(b"C", b"UPDATE 1\0")


@contextmanager
def probe(placement, sql=OBAN_SQL, *, prior=None):
    with socket.socket() as server:
        server.bind(("127.0.0.1", 0))
        server.listen()
        server.settimeout(2)
        predicate = lambda query: query == sql.decode()
        with SettlementProxy(server.getsockname()[1], predicate, placement, prior_query=prior) as proxy:
            yield server, proxy


class SettlementProxyTests(unittest.TestCase):
    def expect_bytes(self, sock, expected):
        read = bytearray()
        while len(read) < len(expected):
            chunk = sock.recv(len(expected) - len(read))
            self.assertTrue(chunk, "connection closed before the expected frames")
            read.extend(chunk)
        self.assertEqual(read, expected)

    def expect_cut(self, sock):
        try:
            self.assertEqual(sock.recv(1), b"")
        except ConnectionResetError:
            pass

    @contextmanager
    def session(self, server, proxy):
        with socket.create_connection(("127.0.0.1", proxy.port), timeout=2) as client:
            database, _ = server.accept()
            with database:
                database.settimeout(2)
                hello = startup()
                client.sendall(hello[:2])
                client.sendall(hello[2:])
                self.expect_bytes(database, hello)
                auth = message(b"R", b"\0\0\0\x0aSCRAM-SHA-256\0\0")
                database.sendall(auth)
                self.expect_bytes(client, auth)
                response = message(b"p", b"synthetic SCRAM bytes")
                client.sendall(response)
                self.expect_bytes(database, response)
                ready = message(b"R", b"\0\0\0\0") + IDLE
                database.sendall(ready[:7])
                database.sendall(ready[7:])
                self.expect_bytes(client, ready)
                yield client, database

    def test_before_commit_drops_a_matching_parse_and_passes_retry(self):
        parse = message(b"P", b"named\0" + OBAN_SQL + b"\0\0\0")
        with probe("before_commit") as (server, proxy):
            with self.session(server, proxy) as (client, database):
                client.sendall(parse[:8])
                client.sendall(parse[8:] + message(b"H"))
                self.expect_cut(database)
                self.expect_cut(client)
            with self.session(server, proxy) as (client, database):
                client.sendall(parse + SYNC + QUERY)
                self.expect_bytes(database, parse + SYNC + QUERY)
                result = message(b"1") + IDLE + COMPLETE + IDLE
                database.sendall(result)
                self.expect_bytes(client, result)
        self.assertEqual(proxy.fault, {
            "placement": "before_commit", "query_forwarded": False,
            "command_complete": False, "ready_status": None,
        })

    def test_commit_cut_is_armed_only_on_the_batch_session(self):
        commit = message(b"Q", b"COMMIT\0")
        claim_sql = b"SELECT * FROM mesh_jobs.claim_batch_v1()"
        claim = message(b"Q", claim_sql + b"\0")
        started = COMPLETE + message(b"Z", b"T")
        committed = message(b"C", b"COMMIT\0") + IDLE
        for placement in ("before_commit", "after_commit"):
            with self.subTest(placement=placement):
                with probe(placement, b"COMMIT", prior=lambda sql: sql == claim_sql.decode()) as (server, proxy):
                    with self.session(server, proxy) as (batch, batch_db):
                        batch.sendall(claim)
                        self.expect_bytes(batch_db, claim)
                        batch_db.sendall(started)
                        self.expect_bytes(batch, started)
                        with self.session(server, proxy) as (other, other_db):
                            other.sendall(commit)
                            self.expect_bytes(other_db, commit)
                            other_db.sendall(committed)
                            self.expect_bytes(other, committed)
                        batch.sendall(commit)
                        if placement == "after_commit":
                            self.expect_bytes(batch_db, commit)
                            batch_db.sendall(committed)
                        self.expect_cut(batch_db)
                        self.expect_cut(batch)
                self.assertEqual(proxy.fault["placement"], placement)

    def test_claim_cut_forwards_commit_command_complete_and_only_drops_ready(self):
        claim_sql = b"SELECT * FROM mesh_jobs.claim_batch_v1()"
        claim = (message(b"P", b"\0" + claim_sql + b"\0\0\0")
                 + message(b"B", b"\0\0\0\0\0\0\0\0")
                 + message(b"E", b"\0\0\0\0\0") + SYNC)
        claimed = message(b"1") + message(b"2") + COMPLETE + message(b"Z", b"T")
        commit = message(b"Q", b"COMMIT\0")
        complete = message(b"C", b"COMMIT\0")
        notice = message(b"N", b"Msynthetic notice\0\0")
        with probe("after_command_complete", b"COMMIT", prior=lambda sql: sql == claim_sql.decode()) as (server, proxy):
            with self.session(server, proxy) as (batch, batch_db):
                batch.sendall(claim)
                self.expect_bytes(batch_db, claim)
                batch_db.sendall(claimed)
                self.expect_bytes(batch, claimed)
                with self.session(server, proxy) as (other, other_db):
                    other.sendall(commit)
                    self.expect_bytes(other_db, commit)
                    other_db.sendall(complete + IDLE)
                    self.expect_bytes(other, complete + IDLE)
                batch.sendall(commit)
                self.expect_bytes(batch_db, commit)
                batch_db.sendall(complete)
                self.expect_bytes(batch, complete)
                batch_db.sendall(notice + IDLE[:-1])
                self.expect_bytes(batch, notice)
                self.assertIsNone(proxy.fault)
                batch_db.sendall(IDLE[-1:])
                self.expect_cut(batch)
                self.expect_cut(batch_db)
            with self.session(server, proxy) as (retry, retry_db):
                retry.sendall(claim + commit)
                self.expect_bytes(retry_db, claim + commit)
                retry_db.sendall(claimed + complete + IDLE)
                self.expect_bytes(retry, claimed + complete + IDLE)
        self.assertEqual(proxy.fault, {
            "placement": "after_command_complete", "query_forwarded": True,
            "command_complete": True, "ready_status": "I",
            "command_complete_forwarded": True, "ready_for_query_forwarded": False,
        })

    def test_claim_cut_requires_a_successful_commit_and_idle_ready(self):
        claim = message(b"Q", b"claim\0")
        commit = message(b"Q", b"COMMIT\0")
        complete = message(b"C", b"COMMIT\0")
        for result in (message(b"C", b"ROLLBACK\0") + IDLE,
                       message(b"E", b"\0") + IDLE,
                       complete + message(b"Z", b"T")):
            with self.subTest(result=result), probe(
                    "after_command_complete", b"COMMIT", prior=lambda sql: sql == "claim") as (server, proxy):
                with self.session(server, proxy) as (client, database):
                    client.sendall(claim + commit)
                    self.expect_bytes(database, claim + commit)
                    claimed = COMPLETE + message(b"Z", b"T")
                    database.sendall(claimed + result)
                    self.expect_bytes(client, claimed + result)
            self.assertIsNone(proxy.fault)

    def test_before_commit_cuts_only_one_of_two_matching_connections(self):
        with probe("before_commit") as (server, proxy):
            with self.session(server, proxy) as (first, first_db):
                with self.session(server, proxy) as (second, second_db):
                    first.sendall(QUERY)
                    second.sendall(QUERY)
                    forwarded = []
                    for index, database in enumerate((first_db, second_db)):
                        try:
                            byte = database.recv(1)
                        except ConnectionResetError:
                            byte = b""
                        if byte:
                            self.assertEqual(byte, b"Q")
                            self.expect_bytes(database, QUERY[1:])
                            database.sendall(COMPLETE + IDLE)
                            forwarded.append(index)
                    self.assertEqual(len(forwarded), 1)
                    clients = (first, second)
                    self.expect_bytes(clients[forwarded[0]], COMPLETE + IDLE)
                    self.expect_cut(clients[1 - forwarded[0]])
        self.assertEqual(proxy.fault["placement"], "before_commit")

    def test_after_commit_preserves_postgrex_prepare_and_drops_execution_result(self):
        parse = message(b"P", b"named\0" + OBAN_SQL + b"\0\0\0")
        prepare = parse + message(b"D", b"Snamed\0") + message(b"H")
        prepared = message(b"1") + message(b"t", b"\0\0") + message(b"n")
        bind = message(b"B", b"\0named\0\0\0\0\0\0\0")
        execute = message(b"E", b"\0\0\0\0\0")
        with probe("after_commit") as (server, proxy):
            with self.session(server, proxy) as (client, database):
                client.sendall(prepare)
                self.expect_bytes(database, prepare)
                database.sendall(prepared)
                self.expect_bytes(client, prepared)
                client.sendall(bind + execute[:3])
                client.sendall(execute[3:] + SYNC)
                self.expect_bytes(database, bind + execute + SYNC)
                database.sendall(message(b"2") + COMPLETE + IDLE[:5])
                self.expect_bytes(client, message(b"2"))
                self.assertIsNone(proxy.fault)
                database.sendall(IDLE[5:])
                self.expect_cut(client)
            with self.session(server, proxy) as (client, database):
                client.sendall(QUERY)
                self.expect_bytes(database, QUERY)
                database.sendall(COMPLETE + IDLE)
                self.expect_bytes(client, COMPLETE + IDLE)
        self.assertEqual(proxy.fault, {
            "placement": "after_commit", "query_forwarded": True,
            "command_complete": True, "ready_status": "I",
        })

    def test_after_commit_hides_the_mesh_result_from_one_extended_batch(self):
        parse = message(b"P", b"\0" + MESH_SQL + b"\0\0\0")
        bind = message(b"B", b"\0\0\0\0\0\0\0\0")
        describe = message(b"D", b"P\0")
        execute = message(b"E", b"\0\0\0\0\0")
        request = parse + bind + describe + execute + SYNC
        metadata = message(b"1") + message(b"2") + message(b"T", b"synthetic metadata")
        result = message(b"D", b"synthetic row") + message(b"C", b"SELECT 1\0")
        with probe("after_commit", MESH_SQL) as (server, proxy):
            with self.session(server, proxy) as (client, database):
                client.sendall(request)
                self.expect_bytes(database, request)
                database.sendall(metadata + result + IDLE)
                self.expect_bytes(client, metadata)
                self.expect_cut(client)
                self.expect_cut(database)
        self.assertTrue(proxy.fault["command_complete"])

    def test_after_commit_preserves_an_earlier_pipelined_response(self):
        previous = message(b"Q", b"SELECT 1\0")
        responded = message(b"C", b"SELECT 1\0") + IDLE
        with probe("after_commit") as (server, proxy):
            with self.session(server, proxy) as (client, database):
                client.sendall(previous + QUERY)
                self.expect_bytes(database, previous + QUERY)
                database.sendall(responded + message(b"D", b"synthetic result") + COMPLETE + IDLE)
                self.expect_bytes(client, responded)
                self.expect_cut(client)
        self.assertEqual(proxy.fault["ready_status"], "I")

    def test_after_commit_requires_success_and_idle_for_the_matched_query(self):
        for result in (message(b"E", b"\0") + IDLE, COMPLETE + message(b"Z", b"T")):
            with self.subTest(result=result), probe("after_commit") as (server, proxy):
                with self.session(server, proxy) as (client, database):
                    client.sendall(QUERY)
                    self.expect_bytes(database, QUERY)
                    database.sendall(result)
                    self.expect_bytes(client, result)
                    self.assertIsNone(proxy.fault)

    def test_rejects_unframed_startup_and_oversized_messages(self):
        with probe("before_commit") as (server, proxy):
            with socket.create_connection(("127.0.0.1", proxy.port), timeout=2) as client:
                database, _ = server.accept()
                with database:
                    database.settimeout(2)
                    client.sendall(struct.pack("!II", 8, 80877103))  # SSLRequest
                    self.expect_cut(database)
                    self.expect_cut(client)
            with self.session(server, proxy) as (client, database):
                client.sendall(b"Q" + struct.pack("!I", 2**31))
                self.expect_cut(database)
                self.expect_cut(client)
            self.assertIsNone(proxy.fault)
