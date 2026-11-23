package main

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/riverqueue/river"
	"github.com/riverqueue/river/riverdriver/riverpgxv5"
	"github.com/riverqueue/river/rivermigrate"
)

func openPool(ctx context.Context, name string, cap int32) (*pgxpool.Pool, error) {
	dsn := os.Getenv("RIVER_BENCH_DATABASE_URL")
	u, err := url.Parse(dsn)
	if err != nil || u.Scheme != "postgresql" && u.Scheme != "postgres" ||
		net.ParseIP(u.Hostname()) == nil || !net.ParseIP(u.Hostname()).IsLoopback() ||
		u.Port() == "" || u.Path == "" || u.Query().Get("sslmode") != "verify-full" ||
		u.Query().Get("sslrootcert") == "" {
		return nil, errors.New("RIVER_BENCH_DATABASE_URL must use a loopback IP, explicit port and database, sslmode=verify-full, and sslrootcert")
	}
	cfg, err := pgxpool.ParseConfig(dsn)
	if err != nil {
		return nil, errors.New("cannot parse RIVER_BENCH_DATABASE_URL with pgx")
	}
	if cfg.ConnConfig.Host != u.Hostname() || len(cfg.ConnConfig.Fallbacks) > 0 ||
		cfg.ConnConfig.TLSConfig == nil || cfg.ConnConfig.TLSConfig.InsecureSkipVerify {
		return nil, errors.New("pgx must use verified TLS to the configured loopback IP without fallback")
	}
	cfg.ConnConfig.RuntimeParams["application_name"] = name
	cfg.ConnConfig.RuntimeParams["timezone"] = "UTC"
	cfg.ConnConfig.TLSConfig.MinVersion = tls.VersionTLS12
	cfg.MaxConns, cfg.MinConns, cfg.MinIdleConns = cap, 0, 0
	pool, err := pgxpool.NewWithConfig(ctx, cfg)
	if err != nil {
		return nil, err
	}
	if err = pool.Ping(ctx); err != nil {
		pool.Close()
		// A connection error may include the database connection settings.
		return nil, fmt.Errorf("verified TLS ping failed for %s (%T)", name, err)
	}
	return pool, nil
}

func databaseIdentity(ctx context.Context, pool *pgxpool.Pool, out string, cap int) error {
	var data struct {
		Version      int    `json:"server_version_num"`
		User         string `json:"user"`
		TLS          bool   `json:"tls"`
		TLSVersion   string `json:"tls_version"`
		SyncCommit   bool   `json:"synchronous_commit"`
		FSync        bool   `json:"fsync"`
		FullPages    bool   `json:"full_page_writes"`
		CommitStamps bool   `json:"track_commit_timestamp"`
		PhysicalCap  int    `json:"max_connections"`
		DatabaseCap  int    `json:"database_connection_limit"`
		Reserved     int    `json:"reserved_connections"`
		SuperReserve int    `json:"superuser_reserved_connections"`
		OtherClients int    `json:"other_client_backends"`
	}
	err := pool.QueryRow(ctx, `
		SELECT current_setting('server_version_num')::int, current_user,
		  s.ssl, coalesce(s.version, ''), current_setting('synchronous_commit') = 'on',
		  current_setting('fsync') = 'on', current_setting('full_page_writes') = 'on',
		  current_setting('track_commit_timestamp') = 'on', current_setting('max_connections')::int,
		  (SELECT datconnlimit FROM pg_database WHERE datname = current_database()),
		  current_setting('reserved_connections')::int, current_setting('superuser_reserved_connections')::int,
		  (SELECT count(*) FROM pg_stat_activity WHERE backend_type = 'client backend' AND pid <> pg_backend_pid())
		FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()`).Scan(&data.Version, &data.User,
		&data.TLS, &data.TLSVersion, &data.SyncCommit, &data.FSync, &data.FullPages, &data.CommitStamps, &data.PhysicalCap,
		&data.DatabaseCap, &data.Reserved, &data.SuperReserve, &data.OtherClients)
	if err != nil {
		return err
	}
	if err = writeJSON(out, data); err != nil {
		return err
	}
	if data.Version != 170011 || !data.TLS || !data.SyncCommit || !data.FSync || !data.FullPages || !data.CommitStamps ||
		data.PhysicalCap != cap || data.DatabaseCap != cap || data.Reserved != 0 ||
		data.SuperReserve != 0 || data.OtherClients != 0 {
		return errors.New("database preflight failed, see database.json")
	}
	return nil
}

// The application relations use the same columns, indexes and constraints as
// the frozen Oban arm, isolated per run schema.
const applicationSQL = `
	CREATE TABLE @.mesh_jobs_parity_attempts (
	    engine text NOT NULL, namespace text NOT NULL, job_key text NOT NULL,
	    effect_key text NOT NULL, attempt integer NOT NULL, entered_at timestamptz NOT NULL,
	    worker_pid integer NOT NULL
	);
	CREATE INDEX ON @.mesh_jobs_parity_attempts (engine, namespace, job_key);
	CREATE TABLE @.mesh_jobs_parity_effects (
	    engine text NOT NULL, namespace text NOT NULL, job_key text NOT NULL,
	    effect_key text NOT NULL, effected_at timestamptz NOT NULL,
	    PRIMARY KEY (engine, namespace, effect_key)
	);
	CREATE TABLE @.mesh_jobs_parity_product (
	    engine text NOT NULL, namespace text NOT NULL, request_id text NOT NULL,
	    PRIMARY KEY (engine, namespace, request_id)
	);
`

func setupSchema(ctx context.Context, pool *pgxpool.Pool, schema string, path string) error {
	_, err := pool.Exec(ctx, "CREATE SCHEMA "+schema+";"+strings.ReplaceAll(applicationSQL, "@", schema))
	if err != nil {
		return err
	}
	migrator, err := rivermigrate.New(riverpgxv5.New(pool), &rivermigrate.Config{Schema: schema})
	if err != nil {
		return err
	}
	migrations, err := migrator.Migrate(ctx, rivermigrate.DirectionUp, nil)
	if err != nil {
		return err
	}
	return writeJSON(path, migrations)
}

type batch struct {
	RequestID string    `json:"request_id"`
	Jobs      []jobArgs `json:"jobs"`
	Rollback  bool      `json:"rollback"`
}

type receipt struct {
	RequestID       string            `json:"request_id"`
	XID             string            `json:"xid"`
	IDs             map[string]string `json:"ids"`
	Commit          *time.Time        `json:"commit"`
	Rollback        bool              `json:"rollback"`
	SubmittedUS     int64             `json:"submitted_us"`
	ReadFinishedUS  int64             `json:"read_finished_us"`
	DecodedUS       int64             `json:"decoded_us"`
	EnqueueCalledUS int64             `json:"enqueue_called_us"`
	InputBytes      int               `json:"input_bytes"`
	ReturnedUS      int64             `json:"returned_us"`
}

func enqueue(ctx context.Context, pool *pgxpool.Pool, client *river.Client[pgx.Tx], schema string, input batch, r *receipt) error {
	r.IDs = make(map[string]string)
	r.Rollback = input.Rollback
	tx, err := pool.Begin(ctx)
	if err != nil {
		return err
	}
	defer tx.Rollback(context.Background())
	if err = tx.QueryRow(ctx, "SELECT pg_current_xact_id()::text").Scan(&r.XID); err != nil {
		return err
	}
	_, err = tx.Exec(ctx, "INSERT INTO "+schema+
		".mesh_jobs_parity_product VALUES ('river', $1, $2)", schema, input.RequestID)
	if err != nil {
		return err
	}
	params := make([]river.InsertManyParams, len(input.Jobs))
	for i, job := range input.Jobs {
		params[i] = river.InsertManyParams{Args: job}
	}
	const batchSize = 128
	var lastID int64
	for i := 0; i < len(params); i += batchSize {
		chunk := params[i:min(i+batchSize, len(params))]
		results, err := client.InsertManyTx(ctx, tx, chunk)
		if err != nil {
			return err
		}
		if len(results) != len(chunk) {
			return fmt.Errorf("insert returned %d rows for %d jobs", len(results), len(chunk))
		}
		for j, result := range results {
			var persistedKey struct{ Key string }
			expectedKey := input.Jobs[i+j].Key
			if err := json.Unmarshal(result.Job.EncodedArgs, &persistedKey); err != nil ||
				persistedKey.Key != expectedKey || result.Job.ID <= lastID {
				return fmt.Errorf("insert did not retain frozen queue order at %s", expectedKey)
			}
			lastID = result.Job.ID
			r.IDs[expectedKey] = strconv.FormatInt(lastID, 10)
		}
	}
	if input.Rollback {
		err = tx.Rollback(ctx)
	} else {
		err = tx.Commit(ctx)
	}
	if err != nil {
		return err
	}
	err = pool.QueryRow(ctx, "SELECT pg_xact_commit_timestamp($1::text::xid8::xid)", r.XID).Scan(&r.Commit)
	if err == nil && (r.Commit == nil) != input.Rollback {
		err = errors.New("enqueue transaction has unexpected commit status")
	}
	return err
}
