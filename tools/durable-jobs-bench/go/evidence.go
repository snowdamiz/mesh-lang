package main

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

type sample struct {
	OffsetMS    float64 `json:"offset_ms"`
	Idle        bool    `json:"idle"`
	TLS         bool    `json:"all_connections_tls"`
	Connections int     `json:"connections"`
	RunningRows int     `json:"database_running_rows"`
	Terminal    int     `json:"terminal"`
	Processes   int     `json:"processes"`
	Threads     int     `json:"threads"`
	RSSBytes    int64   `json:"rss_bytes"`
	HostLoad    float64 `json:"host_load_average_1m"`
}

type observer struct {
	pool    *pgxpool.Pool
	schema  string
	profile profile
	runners []*runner
	sink    *json.Encoder
	start   time.Time
	peak    sample
}

func (o *observer) capture(ctx context.Context, idle bool) (sample, error) {
	s := sample{OffsetMS: ms(time.Since(o.start)), Idle: idle}
	err := o.pool.QueryRow(ctx, `
		WITH live AS (
		    SELECT count(*) AS connections, coalesce(bool_and(s.ssl IS TRUE), false) AS tls
		    FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
		    WHERE a.backend_type = 'client backend'
		)
		SELECT (SELECT connections FROM live), (SELECT tls FROM live),
			count(*) FILTER (WHERE state = 'running'),
			count(*) FILTER (WHERE state IN ('completed', 'cancelled', 'discarded'))
		FROM `+o.schema+`.river_job`).Scan(&s.Connections, &s.TLS, &s.RunningRows, &s.Terminal)
	if err != nil {
		return s, err
	}
	for _, r := range o.runners {
		select {
		case <-r.done:
			return s, fmt.Errorf("runner %d exited during observation: %v", r.cmd.Process.Pid, r.err)
		default:
		}
		s.Processes++
		if err := readProcessStats(r.cmd.Process.Pid, &s); err != nil {
			return s, err
		}
	}
	if data, err := os.ReadFile("/proc/loadavg"); err == nil {
		_, _ = fmt.Sscan(string(data), &s.HostLoad)
	}
	if err := o.sink.Encode(s); err != nil {
		return s, err
	}
	if !s.TLS || s.Connections > o.profile.Cap {
		return s, fmt.Errorf("sample exceeds profile caps: %+v", s)
	}
	o.peak.Connections = max(o.peak.Connections, s.Connections)
	o.peak.RunningRows = max(o.peak.RunningRows, s.RunningRows)
	o.peak.Processes = max(o.peak.Processes, s.Processes)
	o.peak.Threads = max(o.peak.Threads, s.Threads)
	o.peak.RSSBytes = max(o.peak.RSSBytes, s.RSSBytes)
	o.peak.HostLoad = max(o.peak.HostLoad, s.HostLoad)
	return s, nil
}

func (o *observer) waitTerminal(ctx context.Context, count int) error {
	for {
		s, err := o.capture(ctx, false)
		if err != nil || s.Terminal >= count {
			return err
		}
		if err := pause(ctx, observerInterval); err != nil {
			return err
		}
	}
}

func readProcessStats(pid int, s *sample) error {
	if runtime.GOOS != "linux" {
		return nil
	}
	status, err := os.ReadFile(fmt.Sprintf("/proc/%d/status", pid))
	if err != nil {
		return err
	}
	for line := range strings.SplitSeq(string(status), "\n") {
		if value, ok := strings.CutPrefix(line, "Threads:"); ok {
			n, err := strconv.Atoi(strings.TrimSpace(value))
			if err != nil {
				return err
			}
			s.Threads += n
		}
		if value, ok := strings.CutPrefix(line, "VmRSS:"); ok {
			kib, err := strconv.ParseInt(strings.TrimSpace(strings.TrimSuffix(value, " kB")), 10, 64)
			if err != nil {
				return err
			}
			s.RSSBytes += 1024 * kib
		}
	}
	return nil
}

type jobRow struct {
	ID          string    `json:"id"`
	State       string    `json:"state"`
	Attempt     int       `json:"attempt"`
	MaxAttempts int       `json:"max_attempts"`
	Queue       string    `json:"queue"`
	Kind        string    `json:"kind"`
	Key         string    `json:"key"`
	PayloadHash string    `json:"payload_sha256"`
	HoldMS      int       `json:"hold_ms"`
	FailFirst   bool      `json:"fail_first"`
	Claimed     time.Time `json:"claimed"`
	Finalized   time.Time `json:"finalized_at"`
	Settled     time.Time `json:"settled"`
	SettledXID  string    `json:"settled_xid"`
	Errors      []struct {
		Error   string `json:"error"`
		Attempt int    `json:"attempt"`
	} `json:"errors"`
}

type appRow struct {
	Engine    string    `json:"engine"`
	Namespace string    `json:"namespace"`
	Key       string    `json:"job_key"`
	EffectKey string    `json:"effect_key"`
	Attempt   int       `json:"attempt"`
	RequestID string    `json:"request_id"`
	WorkerPID int       `json:"worker_pid"`
	XID       string    `json:"xid"`
	Entered   time.Time `json:"entered_at"`
	Committed time.Time `json:"committed"`
}

type ledger struct {
	Jobs     []jobRow `json:"jobs"`
	Attempts []appRow `json:"attempts"`
	Effects  []appRow `json:"effects"`
	Product  []appRow `json:"product"`
}

func readLedger(ctx context.Context, pool *pgxpool.Pool, schema, path string) (ledger, error) {
	query := strings.ReplaceAll(`
		SELECT json_build_object(
		  'jobs', (SELECT json_agg(json_build_object(
		    'id', id::text, 'state', state, 'attempt', attempt, 'max_attempts', max_attempts,
		    'queue', queue, 'kind', kind, 'key', args->>'key',
		    'payload_sha256', encode(sha256(convert_to(args->>'payload', 'UTF8')), 'hex'),
		    'hold_ms', (args->>'hold_ms')::int, 'fail_first', (args->>'fail_first')::bool,
		    'claimed', attempted_at, 'finalized_at', finalized_at, 'errors', errors,
		    'settled', pg_xact_commit_timestamp(xmin), 'settled_xid', xmin::text) ORDER BY id) FROM @.river_job),
		  'attempts', (SELECT json_agg(row_to_json(t) ORDER BY job_key, attempt) FROM (
		    SELECT a.*, xmin::text AS xid, pg_xact_commit_timestamp(xmin) AS committed
		    FROM @.mesh_jobs_parity_attempts a) t),
		  'effects', (SELECT json_agg(row_to_json(t) ORDER BY job_key) FROM (
		    SELECT e.*, xmin::text AS xid, pg_xact_commit_timestamp(xmin) AS committed
		    FROM @.mesh_jobs_parity_effects e) t),
		  'product', (SELECT json_agg(row_to_json(t) ORDER BY request_id) FROM (
		    SELECT p.*, xmin::text AS xid, pg_xact_commit_timestamp(xmin) AS committed
		    FROM @.mesh_jobs_parity_product p) t))
	`, "@", schema)
	var data json.RawMessage
	var l ledger
	if err := pool.QueryRow(ctx, query).Scan(&data); err != nil {
		return l, err
	}
	if err := writeJSON(path, data); err != nil {
		return l, err
	}
	err := json.Unmarshal(data, &l)
	return l, err
}

type metrics struct {
	Completed        int       `json:"completed"`
	Retried          int       `json:"retried"`
	RolledBack       int       `json:"rolled_back_batches"`
	Duration         float64   `json:"commit_to_durable_seconds"`
	Throughput       float64   `json:"jobs_per_second"`
	PickupP99        float64   `json:"pickup_p99_ms"`
	ClaimP99         float64   `json:"claim_p99_ms"`
	SettlementP99    float64   `json:"settlement_p99_ms"`
	RunnerCPU        float64   `json:"runner_cpu_seconds_including_startup_and_idle"`
	CPUPerJob        float64   `json:"cpu_seconds_per_job"`
	HandlerPeakBound int       `json:"concurrent_handler_upper_bound"`
	Peak             sample    `json:"peak_samples"`
	InputSHA256      string    `json:"input_sha256,omitempty"`
	FirstCommit      time.Time `json:"first_enqueue_commit"`
	LastSettlement   time.Time `json:"last_durable_terminal_commit"`
}

func (l ledger) reconcile(schema string, batches []batch, receipts []receipt) (metrics, error) {
	var m metrics
	product := make(map[string]appRow)
	attempts := make(map[string][]appRow)
	effects := make(map[string]appRow)
	for _, p := range l.Product {
		product[p.RequestID] = p
	}
	for _, a := range l.Attempts {
		attempts[a.Key] = append(attempts[a.Key], a)
	}
	for _, e := range l.Effects {
		effects[e.Key] = e
	}
	var pickup, claim, settlement []float64
	pos, attemptTotal := 0, 0
	for i, b := range batches {
		r := receipts[i]
		if b.Rollback {
			m.RolledBack++
			continue
		}
		p, ok := product[b.RequestID]
		xid, err := strconv.ParseUint(r.XID, 10, 64)
		if err != nil || !ok || p.Engine != "river" || p.Namespace != schema ||
			p.XID != strconv.FormatUint(uint64(uint32(xid)), 10) || r.Commit == nil || !p.Committed.Equal(*r.Commit) {
			return m, fmt.Errorf("%s: product did not commit atomically with enqueue", b.RequestID)
		}
		delete(product, b.RequestID)
		if m.FirstCommit.IsZero() || r.Commit.Before(m.FirstCommit) {
			m.FirstCommit = *r.Commit
		}
		for _, expected := range b.Jobs {
			if pos >= len(l.Jobs) {
				return m, fmt.Errorf("%s: job missing from ledger", expected.Key)
			}
			j := l.Jobs[pos]
			pos++
			a, e := attempts[j.Key], effects[j.Key]
			wantAttempts := 1
			if expected.FailFirst {
				wantAttempts++
				m.Retried++
			}
			if j.Key != expected.Key || j.ID != r.IDs[expected.Key] || j.State != "completed" ||
				j.Attempt != wantAttempts || j.MaxAttempts != 3 || j.Queue != queue || j.Kind != expected.Kind() ||
				j.PayloadHash != fmt.Sprintf("%x", sha256.Sum256([]byte(expected.Payload))) ||
				j.HoldMS != expected.HoldMS || j.FailFirst != expected.FailFirst || j.Finalized.IsZero() ||
				len(a) != wantAttempts || len(j.Errors) != wantAttempts-1 {
				return m, fmt.Errorf("%s: persisted job, payload, order or attempts differ from fixture", expected.Key)
			}
			attemptTotal += len(a)
			if expected.FailFirst && (j.Errors[0].Attempt != 1 || j.Errors[0].Error != "fixture_fail_first" ||
				!e.Committed.Before(a[1].Committed) || a[1].Committed.After(j.Settled)) {
				return m, fmt.Errorf("%s: expected the intentional retry and one deduplicated effect", j.Key)
			}
			for index, row := range a {
				if row.Engine != "river" || row.Namespace != schema || row.EffectKey != j.Key ||
					row.WorkerPID <= 0 || row.Attempt != index+1 || row.Entered.IsZero() ||
					row.XID == "" || !r.Commit.Before(row.Committed) {
					return m, fmt.Errorf("%s: wrong attempt identity or commit", j.Key)
				}
			}
			if e.Engine != "river" || e.Namespace != schema || e.EffectKey != j.Key ||
				e.XID == "" || e.XID == a[0].XID || !a[0].Committed.Before(e.Committed) ||
				!e.Committed.Before(j.Settled) || j.Claimed.IsZero() {
				return m, fmt.Errorf("%s: missing two distinct ordered commits before the terminal commit", j.Key)
			}
			if !expected.FailFirst && e.Committed.Sub(a[0].Committed) < time.Duration(expected.HoldMS-1)*time.Millisecond {
				return m, fmt.Errorf("%s: hold duration below fixture tolerance", j.Key)
			}
			timings := []time.Duration{a[0].Entered.Sub(*r.Commit), j.Claimed.Sub(*r.Commit), j.Settled.Sub(*r.Commit)}
			for _, timing := range timings {
				if timing < -5*time.Millisecond {
					return m, fmt.Errorf("%s: event precedes enqueue commit beyond clock tolerance", j.Key)
				}
			}
			pickup = append(pickup, ms(max(0, timings[0])))
			claim = append(claim, ms(max(0, timings[1])))
			settlement = append(settlement, ms(timings[2]))
			if j.Settled.After(m.LastSettlement) {
				m.LastSettlement = j.Settled
			}
		}
	}
	if pos == 0 || pos != len(l.Jobs) || attemptTotal != len(l.Attempts) ||
		pos != len(l.Effects) || len(product) > 0 || !m.FirstCommit.Before(m.LastSettlement) {
		return m, fmt.Errorf("unexpected aggregate row counts or commit window")
	}
	m.Completed = pos
	m.Duration = m.LastSettlement.Sub(m.FirstCommit).Seconds()
	m.Throughput = float64(pos) / m.Duration
	m.PickupP99, m.ClaimP99, m.SettlementP99 = p99(pickup), p99(claim), p99(settlement)
	return m, nil
}

func ms(d time.Duration) float64 { return float64(d) / float64(time.Millisecond) }

func p99(values []float64) float64 {
	sort.Float64s(values)
	return values[len(values)-1-len(values)/100]
}
