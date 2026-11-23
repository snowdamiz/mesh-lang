package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"path/filepath"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/riverqueue/river"
	"github.com/riverqueue/river/riverdriver/riverpgxv5"
)

const queue = "parity"

type jobArgs struct {
	Key       string `json:"key"`
	Payload   string `json:"payload"`
	HoldMS    int    `json:"hold_ms"`
	FailFirst bool   `json:"fail_first"`
}

func (jobArgs) Kind() string { return "parity" }

func (jobArgs) InsertOpts() river.InsertOpts {
	return river.InsertOpts{Queue: queue, MaxAttempts: 3}
}

type parityWorker struct {
	river.WorkerDefaults[jobArgs]
	pool   *pgxpool.Pool
	schema string
	active atomic.Int32
	peak   atomic.Int32
	calls  atomic.Int64
}

func (w *parityWorker) NextRetry(*river.Job[jobArgs]) time.Time {
	return time.Now().Add(time.Second)
}

func (w *parityWorker) Work(ctx context.Context, job *river.Job[jobArgs]) error {
	entered := time.Now()
	parallel := w.active.Add(1)
	defer w.active.Add(-1)
	for peak := w.peak.Load(); parallel > peak; peak = w.peak.Load() {
		if w.peak.CompareAndSwap(peak, parallel) {
			break
		}
	}
	w.calls.Add(1)
	conn, err := w.pool.Acquire(ctx)
	if err != nil {
		return err
	}
	defer conn.Release()
	// Each Exec is its own autocommit transaction, including when the
	// same physical session is held across the workload's delay.
	_, err = conn.Exec(ctx, fmt.Sprintf(`
		INSERT INTO %s.mesh_jobs_parity_attempts
			(engine, namespace, job_key, effect_key, attempt, entered_at, worker_pid)
		VALUES ('river', $1, $2, $2, $3, $4, $5)`, w.schema),
		w.schema, job.Args.Key, job.Attempt, entered, os.Getpid())
	if err != nil {
		return err
	}
	if !job.Args.FailFirst {
		if err := pause(ctx, time.Duration(job.Args.HoldMS)*time.Millisecond); err != nil {
			return err
		}
	}
	_, err = conn.Exec(ctx, fmt.Sprintf(`
		INSERT INTO %s.mesh_jobs_parity_effects
			(engine, namespace, job_key, effect_key, effected_at)
		VALUES ('river', $1, $2, $2, clock_timestamp()) ON CONFLICT DO NOTHING`, w.schema),
		w.schema, job.Args.Key)
	if err != nil {
		return err
	}
	if job.Args.FailFirst && job.Attempt == 1 {
		if err := pause(ctx, time.Duration(job.Args.HoldMS)*time.Millisecond); err != nil {
			return err
		}
		return errors.New("fixture_fail_first")
	}
	return nil
}

type workerCapacity struct {
	PID   int   `json:"pid"`
	Slots int   `json:"slots"`
	Peak  int   `json:"peak_concurrent_handlers"`
	Calls int64 `json:"handler_calls"`
}

func runWorker(schema string, slots, index int, readyPath string) error {
	signals, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	pool, err := openPool(signals, fmt.Sprintf("river:%s:runner-%d", schema, index), int32(slots+2))
	if err != nil {
		return err
	}
	defer pool.Close()

	workers := river.NewWorkers()
	worker := &parityWorker{pool: pool, schema: schema}
	river.AddWorker(workers, worker)
	client, err := river.NewClient(riverpgxv5.New(pool), &river.Config{
		Schema: schema, ID: fmt.Sprintf("runner-%d", index), Workers: workers,
		Queues:        map[string]river.QueueConfig{queue: {MaxWorkers: slots}},
		FetchCooldown: 5 * time.Millisecond, FetchPollInterval: pollInterval,
		Logger: slog.New(slog.NewJSONHandler(os.Stderr, nil)),
	})
	if err != nil {
		return err
	}
	workerCtx, abort := context.WithCancel(context.Background())
	defer abort()
	if err := client.Start(workerCtx); err != nil {
		return err
	}
	if err := os.WriteFile(readyPath, []byte("READY\n"), 0600); err != nil {
		abort()
		<-client.Stopped()
		return err
	}
	select {
	case <-signals.Done():
	case <-client.Stopped():
		return errors.New("River client stopped before shutdown")
	}
	stopCtx, finish := context.WithTimeout(context.Background(), 15*time.Second)
	defer finish()
	if err := client.Stop(stopCtx); err != nil {
		abort()
		<-client.Stopped()
		return err
	}
	return writeJSON(filepath.Join(filepath.Dir(readyPath), fmt.Sprintf("runner-%d-capacity.json", index)),
		workerCapacity{PID: os.Getpid(), Slots: slots, Peak: int(worker.peak.Load()), Calls: worker.calls.Load()})
}
