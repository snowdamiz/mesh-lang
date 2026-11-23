package main

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/riverqueue/river"
	"github.com/riverqueue/river/riverdriver/riverpgxv5"
)

type profile struct {
	Runners  int    `json:"runners"`
	Slots    int    `json:"slots"`
	Cap      int    `json:"physical_connection_cap"`
	FullWarm bool   `json:"full_unmeasured_warmup"`
	Digest   string `json:"input_sha256"`
}

var profiles = map[string]profile{
	"noop":   {8, 64, 96, true, "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"},
	"slow":   {1, 64, 68, true, "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"},
	"sparse": {1, 1, 5, false, "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5"},
	"short":  {1, 16, 20, false, "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"},
	"large":  {8, 64, 96, true, "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"},
}

const (
	submitBoundary   = "before_batch_read"
	pollInterval     = 250 * time.Millisecond
	observerInterval = 100 * time.Millisecond
	idleInterval     = 3 * time.Second
)

var schemaPattern = regexp.MustCompile(`^parity_[a-f0-9]{16}$`)

func main() {
	if err := dispatch(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func dispatch() error {
	if len(os.Args) == 6 && os.Args[1] == "worker" {
		schema, slotArg, indexArg, ready := os.Args[2], os.Args[3], os.Args[4], os.Args[5]
		slots, slotErr := strconv.Atoi(slotArg)
		index, indexErr := strconv.Atoi(indexArg)
		if !schemaPattern.MatchString(schema) || slotErr != nil || indexErr != nil ||
			slots < 1 || slots > 64 || index < 0 || index > 7 {
			return errors.New("invalid worker configuration")
		}
		return runWorker(schema, slots, index, ready)
	}

	name := flag.String("profile", "", "frozen profile: noop, slow, sparse, short, large")
	input := flag.String("input", "", "directory of frozen batch files")
	out := flag.String("out", "", "new evidence directory")
	single := flag.Bool("single", false, "run exactly one arm for the matrix controller")
	smoke := flag.Bool("smoke", false, "follow the frozen trial with rollback and retry witnesses")
	flag.Parse()
	p, ok := profiles[*name]
	if !ok || *input == "" || *out == "" || flag.NArg() != 0 {
		flag.Usage()
		return errors.New("profile, input, and out are required")
	}
	if err := os.Mkdir(*out, 0700); err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	err := run(ctx, *name, p, *input, *out, *smoke, *single)
	var message string
	if err != nil {
		message = err.Error()
	}
	return errors.Join(err, writeJSON(filepath.Join(*out, "status.json"), map[string]any{
		"ok": err == nil, "error": message, "finished_at": time.Now().UTC(),
	}))
}

func run(ctx context.Context, name string, p profile, input, out string, smoke, single bool) error {
	paths, err := verifyInput(input, p.Digest)
	if err != nil {
		return err
	}
	build, _ := debug.ReadBuildInfo()
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	exeBytes, err := os.ReadFile(exe)
	if err != nil {
		return err
	}
	runtime.GOMAXPROCS(2)
	if err = writeJSON(filepath.Join(out, "config.json"), map[string]any{
		"format_version": 2, "submit_boundary": submitBoundary, "engine": "river", "profile": name, "profile_config": p, "smoke": smoke,
		"binary_sha256": fmt.Sprintf("%x", sha256.Sum256(exeBytes)), "build": build,
		"os": runtime.GOOS, "arch": runtime.GOARCH, "visible_cpus": runtime.NumCPU(), "gomaxprocs": 2,
		"fetch_cooldown_ms": 5, "poll_ms": pollInterval.Milliseconds(), "poll_only": false,
		"idle_ms": idleInterval.Milliseconds(), "sample_ms": observerInterval.Milliseconds(),
		"runner_pool_max": p.Slots + 2*p.Runners, "producer_observer_pool_max": 1,
	}); err != nil {
		return err
	}
	preflight, cancel := context.WithTimeout(ctx, 20*time.Second)
	pool, err := openPool(preflight, "river:producer_observer", 1)
	if err == nil {
		defer pool.Close()
		err = databaseIdentity(preflight, pool, filepath.Join(out, "database.json"), p.Cap)
	}
	cancel()
	if err != nil {
		return err
	}
	if p.FullWarm && !single {
		if err := trial(ctx, pool, p, name == "sparse", paths, filepath.Join(out, "warmup")); err != nil {
			return fmt.Errorf("warmup: %w", err)
		}
	}
	if err := trial(ctx, pool, p, name == "sparse", paths, filepath.Join(out, "measured")); err != nil {
		return fmt.Errorf("frozen trial: %w", err)
	}
	if smoke {
		witnesses := []batch{
			{RequestID: "rollback", Rollback: true, Jobs: []jobArgs{{Key: "rollback", Payload: "fixture"}}},
			{RequestID: "retry", Jobs: []jobArgs{{Key: "retry", Payload: "fixture", HoldMS: 1, FailFirst: true}}},
		}
		fixtureDir := filepath.Join(out, "fault-input")
		if err := os.Mkdir(fixtureDir, 0700); err != nil {
			return err
		}
		var fixturePaths []string
		for _, witness := range witnesses {
			path := filepath.Join(fixtureDir, witness.RequestID+".json")
			if err := writeJSON(path, witness); err != nil {
				return err
			}
			fixturePaths = append(fixturePaths, path)
		}
		if err := trial(ctx, pool, p, false, fixturePaths, filepath.Join(out, "faults")); err != nil {
			return fmt.Errorf("rollback/retry witnesses: %w", err)
		}
	}
	return nil
}

// verifyInput retains only the ordered file paths. Each submission reads and
// decodes its batch after the runners have reached readiness and idled.
func verifyInput(dir, frozenDigest string) ([]string, error) {
	paths, err := filepath.Glob(filepath.Join(dir, "batch_*.json"))
	if err != nil {
		return nil, err
	}
	combined := sha256.New()
	for _, path := range paths {
		file, err := os.Open(path)
		if err != nil {
			return nil, err
		}
		hash := sha256.New()
		_, err = io.Copy(hash, file)
		if err = errors.Join(err, file.Close()); err != nil {
			return nil, err
		}
		_, _ = fmt.Fprintf(combined, "%x", hash.Sum(nil))
	}
	if digest := hex.EncodeToString(combined.Sum(nil)); digest != frozenDigest {
		return nil, fmt.Errorf("frozen input mismatch: %s != %s", digest, frozenDigest)
	}
	return paths, nil
}

func readBatch(path string, r *receipt) (batch, error) {
	var b batch
	r.SubmittedUS = time.Now().UnixMicro()
	data, err := os.ReadFile(path)
	r.ReadFinishedUS = time.Now().UnixMicro()
	if err != nil {
		return b, err
	}
	r.InputBytes = len(data)
	err = json.Unmarshal(data, &b)
	r.DecodedUS = time.Now().UnixMicro()
	if err != nil {
		return b, err
	}
	if b.RequestID != r.RequestID {
		return b, fmt.Errorf("batch file does not match request ID %s", b.RequestID)
	}
	return b, nil
}

func trial(parent context.Context, pool *pgxpool.Pool, p profile, sparse bool, paths []string, out string) (runErr error) {
	ctx, cancel := context.WithTimeout(parent, 300*time.Second)
	defer cancel()
	if err := os.Mkdir(out, 0700); err != nil {
		return err
	}
	schema := "parity_" + hex.EncodeToString(randBytes(8))
	if err := writeJSON(filepath.Join(out, "schema.json"), schema); err != nil {
		return err
	}
	defer func() {
		cleanup, done := context.WithTimeout(context.Background(), 10*time.Second)
		defer done()
		_, err := pool.Exec(cleanup, "DROP SCHEMA IF EXISTS "+schema+" CASCADE")
		runErr = errors.Join(runErr, err)
	}()
	if err := setupSchema(ctx, pool, schema, filepath.Join(out, "migrations.json")); err != nil {
		return err
	}
	client, err := river.NewClient(riverpgxv5.New(nil), &river.Config{Schema: schema, SkipUnknownJobCheck: true})
	if err != nil {
		return err
	}

	var runners []*runner
	stopped := false
	defer func() {
		if !stopped {
			_, err := stopRunners(runners)
			runErr = errors.Join(runErr, err)
		}
	}()
	for i := range p.Runners {
		slots := p.Slots / p.Runners
		if i < p.Slots%p.Runners {
			slots++
		}
		r, err := startRunner(schema, slots, i, out)
		if err != nil {
			return err
		}
		runners = append(runners, r)
	}
	if err := waitReady(ctx, runners); err != nil {
		return err
	}
	durable := false
	defer func() {
		if !durable {
			probe, done := context.WithTimeout(context.Background(), 5*time.Second)
			defer done()
			_, err := readLedger(probe, pool, schema, filepath.Join(out, "partial-ledger.json"))
			runErr = errors.Join(runErr, err)
		}
	}()

	samples, err := os.Create(filepath.Join(out, "samples.jsonl"))
	if err != nil {
		return err
	}
	defer samples.Close()
	receipts, err := os.Create(filepath.Join(out, "enqueue.jsonl"))
	if err != nil {
		return err
	}
	defer receipts.Close()
	sampleJSON, receiptJSON := json.NewEncoder(samples), json.NewEncoder(receipts)
	sampler := observer{pool: pool, schema: schema, profile: p, runners: runners, sink: sampleJSON, start: time.Now()}
	if _, err = sampler.capture(ctx, true); err != nil {
		return err
	}
	if err := pause(ctx, idleInterval); err != nil {
		return err
	}
	if _, err = sampler.capture(ctx, true); err != nil {
		return err
	}

	batches := make([]batch, 0, len(paths))
	committed := make([]receipt, 0, len(paths))
	var count int
	for _, path := range paths {
		r := receipt{RequestID: strings.TrimSuffix(filepath.Base(path), ".json")}
		b, err := readBatch(path, &r)
		if err == nil {
			r.EnqueueCalledUS = time.Now().UnixMicro()
			err = enqueue(ctx, pool, client, schema, b, &r)
		}
		r.ReturnedUS = time.Now().UnixMicro()
		if err == nil && !(r.SubmittedUS <= r.ReadFinishedUS && r.ReadFinishedUS <= r.DecodedUS &&
			r.DecodedUS <= r.EnqueueCalledUS && r.EnqueueCalledUS <= r.ReturnedUS) {
			err = errors.New("batch read, decode, or enqueue timestamps out of order")
		}
		if err == nil && r.Commit != nil {
			if stamp := r.Commit.UnixMicro(); stamp < r.EnqueueCalledUS || stamp > r.ReturnedUS {
				err = errors.New("PostgreSQL enqueue commit outside adapter call window")
			}
		}
		if err != nil {
			return errors.Join(fmt.Errorf("submit %s: %w", r.RequestID, err), receiptJSON.Encode(r))
		}
		if err := receiptJSON.Encode(r); err != nil {
			return err
		}
		batches = append(batches, b)
		committed = append(committed, r)
		if !b.Rollback {
			count += len(b.Jobs)
		}
		if sparse {
			if err := sampler.waitTerminal(ctx, count); err != nil {
				return err
			}
			if err := pause(ctx, pollInterval); err != nil {
				return err
			}
		}
	}
	if err := sampler.waitTerminal(ctx, count); err != nil {
		return err
	}
	stopped = true
	cpu, err := stopRunners(runners)
	if err != nil {
		return err
	}
	ledger, err := readLedger(ctx, pool, schema, filepath.Join(out, "ledger.json"))
	if err != nil {
		return err
	}
	durable = true
	metrics, err := ledger.reconcile(schema, batches, committed)
	if err != nil {
		return err
	}
	metrics.HandlerPeakBound, err = verifyWorkerCapacity(runners, ledger.Attempts, out)
	if err != nil {
		return err
	}
	metrics.RunnerCPU = cpu
	metrics.CPUPerJob = cpu / float64(count)
	metrics.Peak = sampler.peak
	if len(batches) > 0 && !batches[0].Rollback {
		metrics.InputSHA256 = p.Digest
	}
	return writeJSON(filepath.Join(out, "metrics.json"), metrics)
}

func randBytes(size int) []byte {
	buf := make([]byte, size)
	rand.Read(buf)
	return buf
}

func pause(ctx context.Context, duration time.Duration) error {
	if duration == 0 {
		return ctx.Err()
	}
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func writeJSON(path string, data any) error {
	bytes, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(bytes, '\n'), 0600)
}

func verifyWorkerCapacity(runners []*runner, attempts []appRow, out string) (int, error) {
	calls := make(map[int]int64)
	for _, a := range attempts {
		calls[a.WorkerPID]++
	}
	peak := 0
	for i, r := range runners {
		data, err := os.ReadFile(filepath.Join(out, fmt.Sprintf("runner-%d-capacity.json", i)))
		if err != nil {
			return 0, err
		}
		var cap workerCapacity
		if err := json.Unmarshal(data, &cap); err != nil {
			return 0, err
		}
		if cap.PID != r.cmd.Process.Pid || cap.Slots != r.slots || cap.Calls != calls[cap.PID] ||
			cap.Peak < 0 || cap.Peak > r.slots || (cap.Calls > 0) != (cap.Peak > 0) {
			return 0, fmt.Errorf("runner %d exceeded its slots or did not reconcile handler calls: %+v", i, cap)
		}
		delete(calls, cap.PID)
		peak += cap.Peak
	}
	if len(calls) != 0 {
		return 0, errors.New("attempts attributed to an unknown runner")
	}
	return peak, nil
}

type runner struct {
	slots int
	cmd   *exec.Cmd
	done  chan struct{}
	err   error
	ready string
}

func startRunner(schema string, slots, index int, out string) (*runner, error) {
	exe, err := os.Executable()
	if err != nil {
		return nil, err
	}
	name := filepath.Join(out, fmt.Sprintf("runner-%d", index))
	log, err := os.Create(name + ".log")
	if err != nil {
		return nil, err
	}
	defer log.Close()
	r := &runner{slots: slots, done: make(chan struct{}), ready: name + ".ready"}
	r.cmd = exec.Command(exe, "worker", schema, strconv.Itoa(slots), strconv.Itoa(index), r.ready)
	r.cmd.Env = append(os.Environ(), "GOMAXPROCS=2")
	r.cmd.Stdout, r.cmd.Stderr = log, log
	if err := r.cmd.Start(); err != nil {
		return nil, err
	}
	go func() {
		r.err = r.cmd.Wait()
		close(r.done)
	}()
	return r, nil
}

func waitReady(ctx context.Context, runners []*runner) error {
	ready, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	for _, r := range runners {
		for {
			select {
			case <-r.done:
				return fmt.Errorf("runner %d exited before readiness: %v", r.cmd.Process.Pid, r.err)
			default:
			}
			if _, err := os.Stat(r.ready); err == nil {
				break
			}
			if err := pause(ready, 20*time.Millisecond); err != nil {
				return fmt.Errorf("runner %d readiness: %w", r.cmd.Process.Pid, err)
			}
		}
	}
	return nil
}

func stopRunners(runners []*runner) (float64, error) {
	var stopErr error
	var cpu float64
	for _, r := range runners {
		err := r.cmd.Process.Signal(syscall.SIGTERM)
		if err != nil && !errors.Is(err, os.ErrProcessDone) {
			stopErr = errors.Join(stopErr, err)
		}
	}
	deadline, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	for _, r := range runners {
		select {
		case <-r.done:
		case <-deadline.Done():
			_ = r.cmd.Process.Kill()
			<-r.done
		}
		if r.err != nil {
			stopErr = errors.Join(stopErr, fmt.Errorf("runner %d: %w", r.cmd.Process.Pid, r.err))
		}
		if state := r.cmd.ProcessState; state != nil {
			cpu += (state.UserTime() + state.SystemTime()).Seconds()
		}
	}
	return cpu, stopErr
}
