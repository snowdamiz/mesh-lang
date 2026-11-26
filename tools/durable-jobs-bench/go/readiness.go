package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

const readinessEnv = "RIVER_BENCH_POOL_READINESS"

type poolFill struct {
	StartedUS  int64    `json:"started_us"`
	FinishedUS int64    `json:"finished_us"`
	ElapsedUS  int64    `json:"elapsed_us"`
	PIDs       []uint32 `json:"backend_pids"`
	PoolMax    int32    `json:"pool_max"`
}

func prewarmSharedPool(parent context.Context, pool *pgxpool.Pool, count int, out string) error {
	ctx, cancel := context.WithTimeout(parent, 10*time.Second)
	defer cancel()
	start := time.Now()
	fill := poolFill{StartedUS: start.UnixMicro(), PoolMax: pool.Config().MaxConns}
	// Hold distinct physical connections, then release all of them before
	// Client.Start can acquire and detach one for the notifier.
	held := make([]*pgxpool.Conn, 0, count)
	var acquireErr error
	for range count {
		conn, err := pool.Acquire(ctx)
		if err != nil {
			acquireErr = fmt.Errorf("pool fill stopped after %d acquires (%T)", len(held), err)
			break
		}
		held = append(held, conn)
		fill.PIDs = append(fill.PIDs, conn.Conn().PgConn().PID())
	}
	for _, conn := range held {
		conn.Release()
	}
	end := time.Now()
	fill.ElapsedUS = end.Sub(start).Microseconds()
	fill.FinishedUS = end.UnixMicro()
	return errors.Join(acquireErr, writeJSON(filepath.Join(out, "pool-fill.json"), fill))
}

type backendEvidence struct {
	PID       int    `json:"pid"`
	Started   string `json:"backend_start"`
	App       string `json:"application_name"`
	User      string `json:"db_user"`
	Database  string `json:"database"`
	Superuser bool   `json:"superuser"`
	TLS       bool   `json:"tls"`
	Listener  bool   `json:"last_query_listen"`
	Observer  bool   `json:"observer"`
}

type processCPU struct {
	PID         int   `json:"pid"`
	StartTicks  int64 `json:"start_ticks"`
	UserTicks   int64 `json:"user_ticks"`
	SystemTicks int64 `json:"system_ticks"`
	RSSBytes    int64 `json:"rss_bytes"`
	Threads     int   `json:"threads"`
}

type startupPoint struct {
	Point     string            `json:"point"`
	EpochUS   int64             `json:"epoch_us"`
	ElapsedUS int64             `json:"elapsed_us"`
	Rows      map[string]int    `json:"rows"`
	Backends  []backendEvidence `json:"backends"`
	Producer  processCPU        `json:"producer"`
	Runner    *processCPU       `json:"runner,omitempty"`
	Postgres  []processCPU      `json:"postgres_backends"`
}

type startupEvidence struct {
	Mode        string         `json:"mode"`
	ProducerPID int            `json:"producer_pid"`
	RunnerPID   int            `json:"runner_pid"`
	Points      []startupPoint `json:"points"`
	start       time.Time
}

func (e *startupEvidence) observeIdle(ctx context.Context, pool *pgxpool.Pool, schema string) error {
	idle := time.Now()
	if err := e.capture(ctx, pool, schema, "ready", e.start); err != nil {
		return err
	}
	if err := pause(ctx, time.Until(idle.Add(idleInterval/2))); err != nil {
		return err
	}
	if err := e.capture(ctx, pool, schema, "idle_middle", e.start); err != nil {
		return err
	}
	return pause(ctx, time.Until(idle.Add(idleInterval)))
}

func (e *startupEvidence) capture(ctx context.Context, pool *pgxpool.Pool, schema, name string, origin time.Time) error {
	point := startupPoint{Point: name}
	var backends, counts []byte
	q := strings.ReplaceAll(`
		SELECT (
		  SELECT json_agg(json_build_object('pid', a.pid, 'backend_start', a.backend_start,
		    'application_name', a.application_name, 'db_user', a.usename, 'database', a.datname,
		    'superuser', r.rolsuper, 'tls', s.ssl IS TRUE,
		    'last_query_listen', coalesce(left(a.query, 7) = 'LISTEN ', false),
		    'observer', a.pid = pg_backend_pid()) ORDER BY a.pid)
		  FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
		  LEFT JOIN pg_roles r ON r.oid = a.usesysid
		  WHERE a.backend_type = 'client backend'
		), json_build_object('jobs', (SELECT count(*) FROM @.river_job),
		  'attempts', (SELECT count(*) FROM @.mesh_jobs_parity_attempts),
		  'effects', (SELECT count(*) FROM @.mesh_jobs_parity_effects),
		  'products', (SELECT count(*) FROM @.mesh_jobs_parity_product))`, "@", schema)
	if err := pool.QueryRow(ctx, q).Scan(&backends, &counts); err != nil {
		return fmt.Errorf("startup census: %w", err)
	}
	if err := errors.Join(json.Unmarshal(backends, &point.Backends), json.Unmarshal(counts, &point.Rows)); err != nil {
		return err
	}
	now := time.Now()
	point.EpochUS = now.UnixMicro()
	point.ElapsedUS = now.Sub(origin).Microseconds()
	e.Points = append(e.Points, point)
	recorded := &e.Points[len(e.Points)-1]
	var err error
	if recorded.Producer, err = readProcessCPU(e.ProducerPID); err != nil {
		return err
	}
	if e.RunnerPID != 0 {
		process, err := readProcessCPU(e.RunnerPID)
		if err != nil {
			return err
		}
		recorded.Runner = &process
	}
	observer, listeners := 0, 0
	for _, backend := range point.Backends {
		if !backend.TLS || backend.Superuser || backend.User != "parity_subject" || backend.Database != "river" {
			return fmt.Errorf("unexpected startup backend %d", backend.PID)
		}
		if backend.Observer && backend.App == "river:producer_observer" {
			observer++
		} else if backend.App != "river:"+schema+":runner-0" || backend.Observer {
			return fmt.Errorf("unexpected startup backend application name on %d", backend.PID)
		}
		if backend.Listener {
			listeners++
		}
		process, err := readProcessCPU(backend.PID)
		if err != nil {
			return err
		}
		recorded.Postgres = append(recorded.Postgres, process)
	}
	for table, count := range point.Rows {
		if count != 0 {
			return fmt.Errorf("startup changed %s before submit", table)
		}
	}
	if observer != 1 || len(point.Backends) > 20 ||
		(name == "before_runner" && len(point.Backends) != 1) ||
		((name == "idle_middle" || name == "before_submit") && listeners != 1) {
		return fmt.Errorf("unexpected startup client inventory at %s", name)
	}
	if e.Mode == "warm" && name != "before_runner" {
		first := e.Points[1].Backends
		if len(point.Backends) != 18 {
			return fmt.Errorf("expected 18 warm physical clients at %s, observed %d", name, len(point.Backends))
		}
		for i, backend := range point.Backends {
			if backend.PID != first[i].PID || backend.Started != first[i].Started {
				return fmt.Errorf("warm physical clients changed at %s", name)
			}
		}
	}
	return nil
}

func readProcessCPU(pid int) (processCPU, error) {
	result := processCPU{PID: pid}
	data, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return result, err
	}
	// The process name may itself contain spaces or parentheses.
	stat := strings.Fields(string(data)[strings.LastIndexByte(string(data), ')')+1:])
	if len(stat) < 20 {
		return result, fmt.Errorf("short /proc stat for %d", pid)
	}
	for field, destination := range map[int]*int64{11: &result.UserTicks, 12: &result.SystemTicks, 19: &result.StartTicks} {
		value, err := strconv.ParseInt(stat[field], 10, 64)
		if err != nil {
			return result, err
		}
		*destination = value
	}
	var resources sample
	if err := readProcessStats(pid, &resources); err != nil {
		return result, err
	}
	result.RSSBytes, result.Threads = resources.RSSBytes, resources.Threads
	return result, nil
}
