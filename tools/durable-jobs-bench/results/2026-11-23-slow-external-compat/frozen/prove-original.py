#!/usr/bin/env python3
"""Drive real old and new Mesh executables over PostgreSQL's native protocol."""
from contextlib import contextmanager
from pathlib import Path
import json
import os
import selectors
import subprocess
import sys
import tempfile
import time

from screen import LANE, MODE, json_out, parity, scope
from lib import jobs_parity_fault as relay

OLD = LANE / "artifacts/proof-old"
NEW = LANE / "artifacts/proof-new"
CASES = 16

class TruncateBatch(relay._LoopbackProxy):
    def __init__(self, port):
        super().__init__(port)
        self.observed = None

    def _relay(self, client):
        with client, relay.socket.create_connection(("127.0.0.1", self._upstream_port), timeout=5) as backend:
            active = False
            delivered = 0
            parses = []
            with selectors.DefaultSelector() as sel:
                sel.register(client, selectors.EVENT_READ, (backend, relay._Frames(startup=True)))
                sel.register(backend, selectors.EVENT_READ, (client, relay._Frames()))
                while ready := sel.select(5):
                    for key, _ in ready:
                        dest, parser = key.data
                        data = key.fileobj.recv(65536)
                        if not data:
                            return
                        for kind, body, frame in parser.feed(data):
                            if key.fileobj is client and kind == b"P":
                                _, rest = relay._cstring(body)
                                sql = relay._cstring(rest)[0]
                                if b"mesh_jobs.enqueue_batch_v1(" in sql and sql.startswith(b"SELECT ordinal"):
                                    active = True
                                    parses.append(sql.decode())
                            if key.fileobj is backend and active and kind == b"D":
                                if delivered == 1:
                                    # Declare the full DataRow and forward an incomplete body.
                                    dest.sendall(frame[:9])
                                    self.observed = dict(full_rows=delivered, truncated_bytes=9,
                                                         frame_bytes=len(frame), parses=parses)
                                    return
                                delivered += 1
                            dest.sendall(frame)

def specs(prefix, count=CASES, **extra):
    return [dict(worker="Proof",request_key=f"{prefix}-{i}",
                 args={"prefix":prefix,"ordinal":i,"ch":"🍏"},**extra) for i in range(count)]

def invoke(cluster, db, out, label, rows, action="tx", *, binary=NEW, role="parity_producer",
           ns=None, port=None, token=None, want_error=False, live=False):
    file = out/f"{label}.input.json"
    json_out(file,rows)
    env = dict(cluster.env,BULK_TEST_URL=cluster.url(db,role,port),BULK_TEST_NS=ns or label,
               BULK_TEST_SPEC=str(file),BULK_TEST_TOKEN=token or label,
               MESH_SCHEDULER_MIN_WORKERS="2",MESH_SCHEDULER_MAX_WORKERS="2")
    env[MODE]="1"
    if live:
        return subprocess.Popen([str(binary),action],env=env,text=True,
                                stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    result = subprocess.run([str(binary),action],env=env,text=True,capture_output=True,timeout=30)
    json_out(out/f"{label}.result.json",dict(exit=result.returncode,stdout=result.stdout,stderr=result.stderr))
    if want_error:
        assert result.returncode == 1 and not result.stdout.strip(), result
        return result.stderr
    assert result.returncode == 0, result
    if action == "migrate":
        assert result.stdout.strip() == "MIGRATED"
        return
    return json.loads(result.stdout)

def db_rows(cluster, db, ns):
    return json.loads(cluster.sql(db,f"""
       SELECT json_build_object(
          'business', (SELECT coalesce(json_agg(ARRAY[token,encode(body,'hex'),optional::text,xmin::text] ORDER BY token),'[]') FROM bulk_app),
          'rows', (SELECT coalesce(json_agg(json_build_object(
            'id',j.id,'effect',mesh_jobs.effect_key(j),'args',j.args,'queue',j.queue,'state',j.state,
            'position',j.position,'db_ms',j.inserted_at_ms,'due_ms',j.scheduled_at_ms,
            'commit',pg_xact_commit_timestamp(j.xmin),
            'fp',r.fingerprint,'receipt',r.receipt,'receipt_commit',pg_xact_commit_timestamp(r.xmin)
          ) ORDER BY j.position),'[]') FROM mesh_jobs.jobs j LEFT JOIN mesh_jobs.receipts r
          ON r.namespace=j.namespace AND r.receipt->>'id'=j.id::text
          WHERE j.namespace='{ns}'),
          'admission', (SELECT count(*) FROM mesh_jobs.admissions WHERE namespace='{ns}'),
          'queues', (SELECT count(*) FROM mesh_jobs.queues WHERE namespace='{ns}'),
          'receipts', (SELECT count(*) FROM mesh_jobs.receipts WHERE namespace='{ns}'))"""))

def calls(cluster, db):
    return json.loads(cluster.sql(db,"""
      SELECT coalesce(json_object_agg(query, calls),'{}') FROM pg_stat_statements
      WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())
        AND userid=(SELECT oid FROM pg_roles WHERE rolname='parity_producer')"""))

def writers(observations):
    return [sum(v for k,v in observations.items() if k.startswith(prefix))
            for prefix in ("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(", "SELECT mesh_jobs.enqueue(")]

@contextmanager
def trial(cluster, out, name, template):
    db = "bulk_case"
    directory = out/name
    directory.mkdir()
    cluster.sql("postgres", f"CREATE DATABASE {db} WITH TEMPLATE {template} OWNER parity_subject")
    try:
        yield db,directory
    finally:
        json_out(directory/"statements.json",calls(cluster,db))
        cluster.sql("postgres",f"DROP DATABASE {db}")

def check_layout(cluster, db, expected):
    assert cluster.sql(db,"SELECT mesh_jobs.enqueue_batch_layout_v1()") == expected

def setup(cluster, out):
    cluster.prepare_database("bulk_template")
    cluster.sql("bulk_template","""
        CREATE EXTENSION pg_stat_statements;
        CREATE TABLE public.bulk_app(token text PRIMARY KEY, body bytea NOT NULL, optional bytea);
        GRANT INSERT ON public.bulk_app TO parity_producer, parity_subject;
    """)
    invoke(cluster,"bulk_template",out,"old-migrate",[],"migrate",binary=OLD,role="parity_subject")
    scope(cluster,"bulk_template")
    cluster.sql("postgres","CREATE DATABASE bulk_qualified WITH TEMPLATE bulk_template OWNER parity_subject")
    invoke(cluster,"bulk_qualified",out,"new-migrate",[],"migrate",role="parity_subject")
    if int(cluster.sql("postgres","SHOW server_version_num")) // 10000 != 17:
        return
    cluster.sql("bulk_qualified","""
      GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_v1(text,jsonb),
        mesh_jobs.enqueue_batch_layout_v1() TO parity_producer;
    """)
    check_layout(cluster,"bulk_qualified","t")

def cases(cluster,out,legacy_only):
    with trial(cluster,out,"old-schema","bulk_template") as (db,t):
        val = invoke(cluster,db,t,"legacy",specs("legacy"), "pool")
        assert len(val) == CASES and writers(calls(cluster,db)) == [0,CASES]
    if legacy_only:
        with trial(cluster,out,"older-server-new-migration","bulk_qualified") as (db,t):
            assert cluster.sql(db,"SELECT to_regprocedure('mesh_jobs.enqueue_batch_v1(text,jsonb)')") == ""
            assert len(invoke(cluster,db,t,"new",specs("new"),"pool")) == CASES
            assert writers(calls(cluster,db)) == [0,CASES]
        return

    with trial(cluster,out,"qualified-pooled-and-typed","bulk_qualified") as (db,t):
        rows=specs("mixed")
        rows[1].update(queue="future",delay_ms=3600)
        rows[2].update(queue="future",scheduled_at_ms=253402300799998,suspended=True)
        del rows[3]["request_key"]
        receipts=invoke(cluster,db,t,"pooled",rows,"pool",ns="mixed")
        checked=db_rows(cluster,db,"mixed")
        json_out(t/"pooled.ledger.json",checked)
        assert len(checked["rows"]) == CASES and checked["receipts"] == CASES-1
        assert [r["id"] for r in receipts] == [r["id"] for r in checked["rows"]]
        assert [r["position"] for r in checked["rows"]] == list(range(1,CASES+1))
        assert [r["args"]["ordinal"] for r in checked["rows"]] == list(range(CASES))
        assert all(r["effect"] == receipt["effect_key"] for r,receipt in zip(checked["rows"],receipts,strict=True))
        assert checked["rows"][1]["due_ms"]-checked["rows"][1]["db_ms"] == 3600
        assert checked["rows"][1]["state"] == "scheduled"
        assert checked["rows"][2]["state"] == "suspended" and checked["rows"][2]["due_ms"] == 253402300799998
        assert sum(r["receipt"] == receipt for r,receipt in zip(checked["rows"],receipts,strict=True)) == CASES-1
        assert all(r["commit"] == r["receipt_commit"] for r in checked["rows"] if r["receipt"])
        assert writers(calls(cluster,db)) == [1,0]
        replayable=[r for r in rows if "request_key" in r]
        replayable.append(dict(replayable[-1]))
        old=invoke(cluster,db,t,"old-replay",replayable,binary=OLD,ns="mixed",token="same")
        new=invoke(cluster,db,t,"new-replay",replayable,ns="mixed",token="same")
        assert old==new and old==receipts[:3]+receipts[4:]+receipts[-1:]
        assert writers(calls(cluster,db)) == [2,CASES*2]
        persisted=db_rows(cluster,db,"mixed")
        assert persisted["rows"] == checked["rows"] and len(persisted["business"]) == 1
        assert persisted["business"][0][:3] == ["same","00ff80",None]
        json_out(t/"replayed.ledger.json",persisted)

    with trial(cluster,out,"same-request-and-unique-replace","bulk_qualified") as (db,t):
        first=specs("dup")[0]
        duplicate=[first,dict(first),*specs("dup-extra")[:CASES-2]]
        dup=invoke(cluster,db,t,"dup",duplicate)
        assert dup[0] == dup[1] and writers(calls(cluster,db)) == [1,CASES]
        assert db_rows(cluster,db,"dup")["receipts"] == CASES-1
        previous=specs("replace")[0]
        previous["args"]={"match":1,"marker":"old"}
        replacement=dict(worker="Proof",request_key="different",args={"match":1,"marker":"new"},
            unique={"fields":["worker","args"],"keys":["match"],"states":"incomplete"},
            replace={"available":["args"]})
        result=invoke(cluster,db,t,"replace",[previous,replacement,*specs("remainder")[:CASES-2]])
        assert result[0]["id"] == result[1]["id"] and result[1]["conflict"] and not result[1]["inserted"]
        assert cluster.sql(db,"SELECT args->>'marker' FROM mesh_jobs.jobs WHERE namespace='replace' AND args ? 'marker'") == "new"
        assert writers(calls(cluster,db)) == [1,2*CASES]
        json_out(t/"replace.ledger.json",db_rows(cluster,db,"replace"))

    with trial(cluster,out,"fallback-custom-callbacks","bulk_qualified") as (db,t):
        cluster.sql(db,"""
        CREATE TABLE public.bulk_trigger(ordinal integer GENERATED ALWAYS AS IDENTITY, visible integer);
        CREATE FUNCTION public.bulk_callback() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            INSERT INTO public.bulk_trigger(visible) SELECT count(*) FROM mesh_jobs.jobs;
            RETURN NULL;
        END $$;
        CREATE TRIGGER bulk_statement AFTER INSERT ON mesh_jobs.jobs FOR EACH STATEMENT
            EXECUTE FUNCTION public.bulk_callback();
        GRANT INSERT ON public.bulk_trigger TO parity_producer;
        """)
        check_layout(cluster,db,"f")
        value=invoke(cluster,db,t,"callback",specs("callback"))
        visibility=json.loads(cluster.sql(db,"SELECT json_agg(visible ORDER BY ordinal) FROM bulk_trigger"))
        json_out(t/"visible.json",visibility)
        assert len(value)==CASES and visibility==list(range(1,CASES+1))
        assert writers(calls(cluster,db)) == [1,CASES]

    with trial(cluster,out,"fallback-suppressed-jobs-and-receipts","bulk_qualified") as (db,t):
        cluster.sql(db,"""
        CREATE FUNCTION public.bulk_drop() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$;
        CREATE TRIGGER bulk_skip_job BEFORE INSERT ON mesh_jobs.jobs
            FOR EACH ROW WHEN ((NEW.args->>'ordinal')::int = 1) EXECUTE FUNCTION public.bulk_drop();
        CREATE TRIGGER bulk_skip_receipt BEFORE INSERT ON mesh_jobs.receipts
            FOR EACH ROW WHEN (NEW.request_key LIKE '%-2') EXECUTE FUNCTION public.bulk_drop();
        """)
        rows=invoke(cluster,db,t,"suppress",specs("suppress"))
        persisted=db_rows(cluster,db,"suppress")
        assert rows[1]["id"] is None and len(persisted["rows"])==persisted["receipts"]==CASES-1
        assert writers(calls(cluster,db)) == [1,CASES]
        json_out(t/"suppress.ledger.json",persisted)

    with trial(cluster,out,"fallback-storage-rls-version-acl-isolation","bulk_qualified") as (db,t):
        controls=[
            ("storage","ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL",
             "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTENDED"),
            ("rls","ALTER TABLE mesh_jobs.jobs ENABLE ROW LEVEL SECURITY; CREATE POLICY bulk_all ON mesh_jobs.jobs USING (true)",
             "ALTER TABLE mesh_jobs.jobs DISABLE ROW LEVEL SECURITY; DROP POLICY bulk_all ON mesh_jobs.jobs"),
            ("version","UPDATE mesh_jobs.schema_version SET version=2","UPDATE mesh_jobs.schema_version SET version=1"),
            ("acl","REVOKE EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_v1(text,jsonb) FROM parity_producer",
             "GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_v1(text,jsonb) TO parity_producer")
        ]
        for label,prepare,restore in controls:
            cluster.sql(db,prepare)
            check_layout(cluster,db,"t" if label=="acl" else "f")
            assert len(invoke(cluster,db,t,label,specs(label)))==CASES
            cluster.sql(db,restore)
        assert len(invoke(cluster,db,t,"repeatable",specs("repeatable"),"repeatable"))==CASES
        assert writers(calls(cluster,db))==[3,5*CASES]

    with trial(cluster,out,"savepoint-and-outer-rollback","bulk_qualified") as (db,t):
        rollback=invoke(cluster,db,t,"rollback",specs("rollback"),"rollback")
        assert len(rollback)==CASES
        zero=dict(business=[],rows=[],queues=0,admission=0,receipts=0)
        assert db_rows(cluster,db,"rollback")==zero
        bad=specs("bad")
        bad[-1]["workr"]="unknown"
        pos_before=cluster.sql(db,"SELECT last_value,is_called FROM mesh_jobs.jobs_position_seq")
        assert "did not match any variant" in invoke(cluster,db,t,"invalid",bad,"expect-error")["error"]
        assert cluster.sql(db,"SELECT last_value,is_called FROM mesh_jobs.jobs_position_seq")==pos_before
        assert not db_rows(cluster,db,"invalid")["rows"]
        # A server rejection after the queue initializer is rolled back at the savepoint.
        cluster.sql(db,"REVOKE INSERT (priority) ON mesh_jobs.jobs FROM parity_producer")
        failed=invoke(cluster,db,t,"denied",specs("denied"),"expect-error")
        assert "permission denied for table jobs" in failed["error"]
        state=db_rows(cluster,db,"denied")
        assert {k:v for k,v in state.items() if k!="business"}=={k:v for k,v in zero.items() if k!="business"}
        assert [r[0] for r in state["business"]]==["denied","denied-after","invalid","invalid-after"]
        json_out(t/"outer.ledger.json",state)

    with trial(cluster,out,"concurrent-old-new","bulk_qualified") as (db,t):
        new=invoke(cluster,db,t,"hold-new",specs("new"),"hold",ns="shared",live=True)
        old=None
        try:
            assert new.stdout.readline().strip()=="HELD"
            query=[dict(worker="Proof",args={"prefix":"new"},unique={"fields":["worker","args"],"keys":["prefix"]})]
            old=invoke(cluster,db,t,"wait-old",query,binary=OLD,ns="shared",live=True)
            waiting=parity.wait_for(lambda: cluster.sql(db,"""
                SELECT wait_event_type FROM pg_stat_activity
                WHERE application_name='wait-old' AND wait_event_type='Lock'"""),
                time.monotonic()+5,"old producer waits behind new namespace fence")
            assert waiting=="Lock"
            newout,newerr=new.communicate("commit\n",timeout=10)
            oldout,olderr=old.communicate(timeout=10)
            json_out(t/"concurrent.json",dict(new=[new.returncode,newout,newerr],old=[old.returncode,oldout,olderr],wait=waiting))
            assert new.returncode==old.returncode==0
            new_ids={r["id"] for r in json.loads(newout)}
            result=json.loads(oldout)[0]
            assert result["conflict"] and result["id"] in new_ids
            assert len(db_rows(cluster,db,"shared")["rows"])==CASES
            assert writers(calls(cluster,db))==[1,1]
        finally:
            for proc in (new,old):
                if proc and proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=5)

    for placement in ("before_commit","after_commit","after_command_complete","truncated"):
        with trial(cluster,out,placement,"bulk_qualified") as (db,t):
            rows=specs("cut")
            if placement=="truncated":
                proxy=TruncateBatch(cluster.port)
            else:
                proxy=relay.SettlementProxy(cluster.port,lambda sql:sql.strip().upper()=="COMMIT",placement,
                                            prior_query=lambda sql:sql.startswith("SELECT ordinal, receipt"))
            with proxy:
                err=invoke(cluster,db,t,"cut",rows,ns="cut",token="cut",port=proxy.port,want_error=True)
                assert ("read query" if placement=="truncated" else "read COMMIT") in err, err
                evidence=parity.wait_for(lambda:getattr(proxy,"observed",None) or getattr(proxy,"fault",None),
                                         time.monotonic()+5,"wire cutoff")
            first=db_rows(cluster,db,"cut")
            first_committed=placement in ("after_commit","after_command_complete")
            assert len(first["rows"])==first["receipts"]==(CASES if first_committed else 0)
            assert len(first["business"])==int(first_committed)
            pre_calls=writers(calls(cluster,db))
            assert pre_calls[1]==0 and pre_calls[0] in (0,1),pre_calls
            replay=invoke(cluster,db,t,"replay",rows,ns="cut",token="cut")
            durable=db_rows(cluster,db,"cut")
            assert len(durable["rows"])==durable["receipts"]==len(replay)==CASES and len(durable["business"])==1
            assert [r["id"] for r in replay]==[r["id"] for r in durable["rows"]]
            assert not first_committed or durable==first
            assert writers(calls(cluster,db))[1]==(CASES if first_committed else 0)
            json_out(t/"fault.json",dict(wire=evidence,after_cut=first,after_replay=durable))
            if placement=="after_commit":
                invoke(cluster,db,t,"drained",[],"drain",ns="cut",role="parity_subject")
                assert cluster.sql(db,"SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='cut' AND state='completed' AND attempt=1")==str(CASES)
                assert cluster.sql(db,"SELECT count(*) FROM bulk_app WHERE token <> 'cut' AND body=decode('00ff80','hex') AND optional IS NULL")==str(CASES)
                json_out(t/"native-worker.ledger.json",db_rows(cluster,db,"cut"))

def main(label):
    out=LANE/"evidence"/label
    out.mkdir()
    with tempfile.TemporaryDirectory(dir=LANE/"tmp",prefix="prove-") as d:
        cluster=parity.Cluster(Path(d),out,16)
        try:
            cluster.start()
            version=int(cluster.sql("postgres","SHOW server_version_num"))
            json_out(out/"manifest.json",dict(version=version,artifacts={p.name:parity.digest(p) for p in (OLD,NEW)}))
            setup(cluster,out)
            cases(cluster,out,version//10000!=17)
            (out/"PASS").write_text("compiled producer and PostgreSQL receipts reconciled\n")
        finally:
            cluster.stop()

if __name__=="__main__":
    main(sys.argv[1])
