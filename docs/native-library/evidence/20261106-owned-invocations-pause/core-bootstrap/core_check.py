"""Owned local TLS peers driving actual compiled MobileCore exports.

Python owns orchestration/HTTP fixtures only. Mesh signs and verifies directory
proofs, admits lookups, stores encrypted state, and performs anchor decisions.
"""
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import select
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import threading
import urllib.request


def sha(data):
    return hashlib.sha256(data).hexdigest()


def vector(value):
    return struct.pack('>I', len(value)) + value


def fields(data):
    values = []
    while data:
        assert len(data) >= 4, 'truncated ABI vector'
        size = struct.unpack('>I', data[:4])[0]
        assert len(data) >= size + 4, 'truncated ABI value'
        values.append(data[4:4 + size])
        data = data[4 + size:]
    assert values and values[0] == struct.pack('>I', len(values) - 1), 'invalid ABI list'
    return values[1:]


def companion(security, rows):
    assert 3 <= len(rows) <= 8
    assert sum(len(pem) for _, pem in rows) <= 65536
    frame = b'\x01' + bytes([len(rows)]) + hashlib.sha256(security).digest()
    for origin, pem in sorted(rows):
        encoded = origin.encode('ascii')
        frame += struct.pack('>H', len(encoded)) + encoded + vector(pem)
    assert len(frame) <= 82002
    return frame


class Native:
    def __init__(self, library, root, custody):
        environment = {k: v for k, v in os.environ.items() if 'proxy' not in k.lower()}
        environment['BOOTSTRAP_FIXTURE'] = str(root)
        self.stderr = (root / (custody.name + '.stderr')).open('ab')
        self.process = subprocess.Popen([sys.executable, str(Path(__file__).with_name('driver') / 'native_host.py'),
                                         str(library), str(root), str(custody)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
                                        env=environment)
        self.lock = threading.Lock()
        assert self.read(20) == {'ready': True}

    def read(self, timeout):
        assert select.select([self.process.stdout], [], [], timeout)[0], 'native ABI deadline'
        line = self.process.stdout.readline()
        assert line, f'native host exited {self.process.poll()}'
        return json.loads(line)

    def result(self, symbol, data):
        with self.lock:
            self.process.stdin.write((json.dumps({'symbol': symbol, 'hex': data.hex()}) + '\n').encode())
            self.process.stdin.flush()
            response = self.read(60)
        assert not response['host_errors'], response
        return response['code'], bytes.fromhex(response['hex'])

    def invoke(self, symbol, data, rejected=False):
        code, result = self.result(symbol, data)
        assert code == (9 if rejected else 0), (symbol, code, result)
        return result

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.stderr.close()
        assert self.process.returncode == 0, f'host shutdown failed: {self.process.returncode}'


def certificates(root):
    root.mkdir(mode=0o700)
    def run(*args):
        subprocess.run(['openssl', *map(str, args)], check=True, capture_output=True, timeout=20)
    (root / 'ca.cnf').write_text('[req]\ndistinguished_name=dn\n[dn]\n[v3_ca]\n'
        'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n'
        'subjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n')
    run('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', root / 'ca.key',
        '-out', root / 'ca.pem', '-days', '1', '-subj', '/CN=Morse disposable local CA',
        '-config', root / 'ca.cnf', '-extensions', 'v3_ca')
    run('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', root / 'server.key',
        '-out', root / 'server.csr', '-subj', '/CN=127.0.0.1')
    (root / 'extensions.cnf').write_text('basicConstraints=critical,CA:FALSE\n'
        'keyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n'
        'subjectAltName=IP:127.0.0.1\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n')
    run('x509', '-req', '-in', root / 'server.csr', '-CA', root / 'ca.pem',
        '-CAkey', root / 'ca.key', '-CAcreateserial', '-out', root / 'server.pem',
        '-days', '1', '-extfile', root / 'extensions.cnf')
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(root / 'server.pem', root / 'server.key')
    return context, (root / 'ca.pem').read_bytes()


def run(work, library, scheduled):
    root = work / 'local'
    root.mkdir(mode=0o700)
    requests, handshakes, errors, checks = [], [], [], []
    state = {'scenario': 'setup', 'redirect': None, 'barrier': None, 'rpc_mode': 'valid'}
    report = {'status': 'FAIL', 'checks': checks, 'scope': 'LOCAL compiled MobileCore C ABI',
              'not_run': ['native platform packages and phone UI', 'production/funded/external RPC',
                          'production C ABI next-day clock boundary',
                          'bounded native context path (owned by production context tests)',
                          'pause exactly after receive/before dispatch inside Mesh',
                          'next-due retained-mismatch retry under production wall clock']}
    fixture_host = None
    lock = threading.Lock()

    def event(collection, row):
        with lock:
            row = {'scenario': state['scenario'], **row}
            collection.append(row)
            with (root / 'http-events.jsonl').open('a') as log:
                log.write(json.dumps(row) + '\n')
        return row

    class Server(ThreadingHTTPServer):
        daemon_threads = True
        def get_request(self):
            connection, address = super().get_request()
            if self.tls is None:
                return connection, address
            try:
                connection.settimeout(5)
                secured = self.tls.wrap_socket(connection, server_side=True)
                event(handshakes, {'peer': self.index, 'tls': 'accepted'})
                return secured, address
            except OSError as error:
                event(handshakes, {'peer': self.index, 'tls': 'refused', 'error': type(error).__name__})
                connection.close()
                raise

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, code, payload, location=None):
            self.send_response(code)
            self.send_header('Content-Length', str(len(payload)))
            if location:
                self.send_header('Location', location)
            self.end_headers()
            try:
                self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def do_GET(self):
            event(requests, {'peer': self.server.index, 'path': self.path, 'method': 'GET'})
            self.reply(200 if self.path == '/ready' else 404, b'bootstrap-local-ready')

        def account(self, address):
            if address == (root / 'ring-address').read_text():
                entry = bytes.fromhex((root / 'ring-entry.hex').read_text())
                if state['rpc_mode'] == 'mismatch':
                    entry = entry[:16] + bytes([entry[16] ^ 1]) + entry[17:]
                raw = bytes.fromhex((root / 'header.hex').read_text()) + entry
            else:
                assert address == (root / 'log-address').read_text(), address
                raw = bytes.fromhex((root / 'log.hex').read_text())
                if state['rpc_mode'] == 'slashed':
                    raw = raw[:5] + b'\x01' + raw[6:]
            return raw

        @staticmethod
        def account_json(raw):
            return {'data': [base64.b64encode(raw).decode(), 'base64'],
                    'owner': '1' * 32, 'executable': False, 'lamports': 1, 'rentEpoch': 0}

        def do_POST(self):
            try:
                assert not self.headers.get('Authorization') and not self.headers.get('Cookie')
                size = int(self.headers['Content-Length'])
                assert 0 < size <= 600000
                body = self.rfile.read(size)
                row = {'peer': self.server.index, 'path': self.path, 'method': 'POST',
                       'request_sha256': sha(body)}
                if self.server.index == 'directory':
                    if self.path == '/v2/devices/resolve':
                        payload = fixture_host.invoke('bootstrap_lookup_response', body)
                    else:
                        assert self.path == '/v2/transparency/checkpoint', self.path
                        assert body == struct.pack('>Q', 1), body.hex()
                        row['proof_sequence'] = 1
                        payload = bytes.fromhex((root / 'checkpoint.hex').read_text())
                else:
                    assert self.path == f'/rpc/{self.server.index}?owned=1', self.path
                    request = json.loads(body)
                    assert request['jsonrpc'] == '2.0' and request['id'] == 1, request
                    row['rpc'] = request
                    if request['method'] == 'getBlockTime':
                        assert request['params'] == [500], request
                        result = int((root / 'block-time').read_text())
                    else:
                        assert request['method'] in ('getAccountInfo', 'getMultipleAccounts')
                        addresses, options = request['params']
                        assert options['commitment'] == 'finalized' and options['encoding'] == 'base64'
                        assert 0 <= options['minContextSlot'] <= 501
                        if request['method'] == 'getAccountInfo':
                            raw = self.account(addresses)
                            offset, length = options['dataSlice']['offset'], options['dataSlice']['length']
                            assert (offset, length) in ((0, 64), (0, 352), (64, 104))
                            value = self.account_json(raw[offset:offset + length])
                        else:
                            assert addresses == [(root / 'ring-address').read_text(),
                                                 (root / 'log-address').read_text()]
                            value = [self.account_json(self.account(address)) for address in addresses]
                        result = {'context': {'slot': 501}, 'value': value}
                    payload = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
                row = event(requests, row)
                barrier = state['barrier']
                if barrier and barrier['matches'](row) and not barrier['entered'].is_set():
                    barrier['entered'].set()
                    assert barrier['release'].wait(20), 'HTTP barrier release deadline'
                    if barrier['fail']:
                        # A transport error, not an application-level 503, exercises
                        # Http.send's Err branch and its postflight currency guard.
                        self.close_connection = True
                        self.connection.shutdown(socket.SHUT_RDWR)
                        return
                if state['redirect'] and self.server.index != 'directory':
                    self.reply(307, b'', state['redirect'])
                else:
                    self.reply(200, payload)
            except Exception as error:
                errors.append(repr(error))
                self.reply(500, b'fixture failure')

    def passed(name, **values):
        checks.append({'name': name, 'status': 'PASS', **values})
        print(json.dumps(checks[-1]), flush=True)

    try:
        with ExitStack() as stack:
            contexts, roots = [], []
            for i in range(4):
                context, pem = certificates(root / f'pki-{i}')
                contexts.append(context)
                roots.append(pem)
            (root / 'ca.pem').write_bytes(b''.join(roots[:3]))
            (root / 'wrong-ca.pem').write_bytes(roots[3])
            servers = []
            for index in ['directory', 0, 1, 2, 'redirect-target']:
                server = Server(('127.0.0.1', 0), Handler)
                server.index = index
                server.tls = None if index == 'directory' else contexts[index if isinstance(index, int) else 0]
                thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .02}, daemon=True)
                thread.start()
                stack.callback(server.server_close)
                stack.callback(server.shutdown)
                servers.append(server)
            directory = f'http://127.0.0.1:{servers[0].server_port}'
            origins = [f'https://127.0.0.1:{s.server_port}' for s in servers[1:4]]
            endpoints = [f'{origin}/rpc/{i}?owned=1' for i, origin in enumerate(origins)]
            (root / 'rpc-origins').write_text('\n'.join(endpoints))
            (root / 'network').write_text('\n'.join(['2', '1', directory, directory, directory,
                                                    directory.replace('http:', 'ws:') + '/v1/mailbox/stream', '1']))
            (root / 'lookup-mode').write_text('valid')
            fixture_host = Native(library, root, root / 'fixture-custody')
            stack.callback(fixture_host.close)
            canary = str(root / 'synthetic-canary.db').encode()
            fixture_host.invoke('mesh_messenger_create_account', vector(canary) + vector(b'morse-canary-1'))
            (root / 'entry.hex').write_text(fixture_host.invoke('mesh_messenger_directory_entry', canary).hex())
            devices = fixture_host.invoke('bootstrap_generate', str(root).encode())
            security = (root / 'security').read_bytes()
            good = companion(security, list(zip(origins, roots)))
            (root / 'rpc-tls').write_bytes(good)
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(root / 'ca.pem'))))
            for origin in [directory, *origins]:
                with opener.open(origin + '/ready', timeout=3) as response:
                    assert response.read() == b'bootstrap-local-ready'
            passed('three independently rooted owned TLS peers ready', origins=origins,
                   root_sha256=[sha(p) for p in roots[:3]], frame_sha256=sha(good))

            def new_client(name, frame=good):
                state['scenario'] = name
                owned = root / name
                owned.mkdir(mode=0o700)
                for resource in ('security', 'network', 'ohttp', 'ca.pem', 'wrong-ca.pem'):
                    shutil.copy2(root / resource, owned / resource)
                if frame is not None:
                    (owned / 'rpc-tls').write_bytes(frame)
                client = Native(library, owned, owned / 'custody')
                stack.callback(client.close)
                path = str(owned / 'client.db').encode()
                client.invoke('mesh_messenger_create_account', vector(path) + vector(b'bootstrap-reader'))
                return client, path, owned

            def lookup(client, path):
                return client.invoke('mesh_messenger_directory_resolve', vector(path) + vector(b'morse-canary-1'))

            def rpc_rows(start=0):
                return [row for row in requests[start:] if 'rpc' in row]

            from owned_native import Runtime, OwnedNative
            runtime = Runtime(library, scheduled)
            stack.callback(runtime.close)
            for failure in [False, True]:
                owned = root / ('owned-error' if failure else 'owned-success')
                owned.mkdir()
                for resource in ('security','network','ohttp','rpc-tls'):
                    shutil.copy2(root/resource,owned/resource)
                client = OwnedNative(runtime,owned,owned/'custody')
                path = str(owned/'client.db').encode()
                expected = client.invoke('mesh_messenger_create_account',vector(path)+vector(b'bootstrap-reader'))
                assert client.invoke('mesh_messenger_load_profile',path)==expected
                cache = client.invoke('bootstrap_cache_snapshot',path)
                owner = client.owner()
                barrier = {'entered':threading.Event(),'release':threading.Event(),'matches':lambda row:'rpc' in row,'fail':failure}
                state['barrier']=barrier
                with ThreadPoolExecutor(max_workers=1) as executor:
                    future=executor.submit(client.result,'mesh_messenger_directory_resolve',vector(path)+vector(b'morse-canary-1'),owner)
                    assert barrier['entered'].wait(10),'actual Core TLS RPC barrier missing'
                    started=__import__('time').monotonic()
                    runtime.retire(owner[0])
                    before_requests=len(requests);before_mutations=len(client.mutations)
                    loaded=client.result('mesh_messenger_load_profile',path)
                    status=client.result('mesh_messenger_push_status',path)
                    elapsed=__import__('time').monotonic()-started
                    pending=not future.done()
                    barrier['release'].set()
                    old=future.result(timeout=10)
                state['barrier']=None
                runtime.drop_auth(owner[1]);runtime.drop_node(owner[0])
                facts={'failure':failure,'loaded_status':loaded[0],'profile_exact':loaded[1]==expected,'status_status':status[0],'status_exact':status[1]==b'disabled','old_still_pending_before_release':pending,'elapsed_seconds':elapsed,'old_status':old[0],'old_private_bytes':len(old[1]),'requests_after_release':len(requests)-before_requests,'mutations_after_retire':len(client.mutations)-before_mutations,'cache_unchanged':client.invoke('bootstrap_cache_snapshot',path)==cache}
                report['checks'].append(facts)
                print(json.dumps(facts),flush=True)
                assert pending and elapsed<2.5,'RPC timeout invalidates held-RPC proof'
                assert old==(10,b'') and facts['requests_after_release']==0 and facts['cache_unchanged']
                assert loaded==(0,expected) and status==(0,b'disabled'),'fresh Core load/status cannot finish while old RPC held'
            report['status']='PASS_LOCAL_CORE_OWNED_INVOCATIONS'
    except BaseException as error:
        report['error'] = repr(error)
        raise
    finally:
        report.update(rpc_http_requests=sum('rpc' in row for row in requests),
                      directory_requests=sum(row['peer'] == 'directory' for row in requests),
                      tls_handshakes=handshakes, server_errors=errors,
                      lookup_previous_sizes=((root / 'lookup-history').read_text().splitlines()
                                             if (root / 'lookup-history').exists() else []),
                      public_fixture_hashes={str(p.relative_to(root)): sha(p.read_bytes())
                          for p in root.rglob('*') if p.is_file() and
                          (p.suffix == '.hex' or p.name in ('security', 'network', 'ohttp', 'rpc-tls',
                                                           'ca.pem', 'http-events.jsonl', 'resource-reads.jsonl'))})
        (work / 'local-result.json').write_text(json.dumps(report, indent=2) + '\n')
    return report

if __name__=="__main__":
    run(Path(sys.argv[1]),Path(sys.argv[2]),"--scheduled" in sys.argv)
