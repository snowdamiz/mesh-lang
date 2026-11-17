package main

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strconv"
)

type result struct {
	Ops       int64 `json:"ops"`
	ElapsedNS int64 `json:"elapsed_ns"`
	Checksum  int64 `json:"checksum"`
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	if _, ok := os.LookupEnv("PERF_WORKERS"); ok {
		workers, err := envInt("PERF_WORKERS", 1)
		if err != nil {
			return err
		}
		runtime.GOMAXPROCS(workers)
	}

	var output any
	var err error
	mode := os.Getenv("PERF_MODE")
	switch mode {
	case "actor", "cpu":
		iters, err := envInt("PERF_ITERS", 0)
		if err != nil {
			return err
		}
		var warmup int
		if _, ok := os.LookupEnv("PERF_WARMUP"); ok {
			warmup, err = envInt("PERF_WARMUP", 0)
			if err != nil {
				return err
			}
		}
		if mode == "actor" {
			pairs, err := envInt("PERF_PAIRS", 1)
			if err != nil {
				return err
			}
			stats, err := runActor(pairs, iters, warmup)
			if err != nil {
				return err
			}
			output = stats
		} else {
			stats, err := runCPU(os.Getenv("PERF_FIXTURES"), iters, warmup)
			if err != nil {
				return err
			}
			output = stats
		}
	case "recovery":
		output, err = runRecovery()
	case "serve":
		port, err := envInt("PERF_PORT", 1)
		if err != nil {
			return err
		}
		if port > 65535 {
			return fmt.Errorf("PERF_PORT must be between 1 and 65535")
		}
		return serve(port)
	default:
		return fmt.Errorf("unknown PERF_MODE %q", mode)
	}
	if err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(output)
}

func envInt(name string, minimum int) (int, error) {
	value, err := strconv.Atoi(os.Getenv(name))
	if err != nil || value < minimum {
		return 0, fmt.Errorf("%s must be a base-ten integer >= %d", name, minimum)
	}
	return value, nil
}
