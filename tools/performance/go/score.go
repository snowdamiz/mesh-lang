package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"os"
	"strconv"
	"time"
)

type jsonInteger int64

func (n *jsonInteger) UnmarshalJSON(data []byte) error {
	value, err := strconv.ParseInt(string(data), 10, 64)
	if err != nil {
		number, err := strconv.ParseFloat(string(data), 64)
		if err != nil || number < -1_000_000 || number > 1_000_000 || number != math.Trunc(number) {
			return errors.New("expected a bounded integral JSON number")
		}
		value = int64(number)
	}
	if value < -1_000_000 || value > 1_000_000 {
		return errors.New("number outside score bounds")
	}
	*n = jsonInteger(value)
	return nil
}

type record struct {
	id     jsonInteger
	values []jsonInteger
}

var warmupResultSink int64

func (r *record) UnmarshalJSON(data []byte) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	var id jsonInteger
	if err := json.Unmarshal(fields["id"], &id); err != nil || id < 0 || id > 1_000_000 {
		return errors.New("id must be an integer between 0 and 1000000")
	}
	var values []jsonInteger
	if err := json.Unmarshal(fields["values"], &values); err != nil || values == nil || len(values) > 256 {
		return errors.New("values must be an array of up to 256 integers")
	}
	*r = record{id: id, values: values}
	return nil
}

func (r record) score() int64 {
	score := int64(r.id)
	for index, value := range r.values {
		score += (int64(index) + 1) * int64(value)
	}
	return score
}

func runCPU(path string, iters, warmup int) (result, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return result{}, err
	}
	var records []record
	if err := json.Unmarshal(data, &records); err != nil {
		return result{}, fmt.Errorf("decode fixtures: %w", err)
	}
	if records == nil || (len(records) == 0 && (iters > 0 || warmup > 0)) {
		return result{}, errors.New("fixtures must be a JSON array with records for the requested operations")
	}
	warmupResultSink = scoreRecords(records, warmup)
	started := time.Now()
	checksum := scoreRecords(records, iters)
	elapsed := time.Since(started).Nanoseconds()
	return result{Ops: int64(iters), ElapsedNS: elapsed, Checksum: checksum}, nil
}

func scoreRecords(records []record, iters int) int64 {
	var checksum int64
	index := 0
	for range iters {
		checksum += records[index].score()
		index++
		if index == len(records) {
			index = 0
		}
	}
	return checksum
}

func serve(port int) error {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, struct {
			OK bool `json:"ok"`
		}{OK: true})
	})
	mux.HandleFunc("POST /score", func(w http.ResponseWriter, r *http.Request) {
		data, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1<<20))
		var input record
		if err != nil || json.Unmarshal(data, &input) != nil {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusBadRequest)
			io.WriteString(w, `{"error":"invalid record"}`)
			return
		}
		writeJSON(w, struct {
			ID    int64 `json:"id"`
			Score int64 `json:"score"`
		}{ID: int64(input.id), Score: input.score()})
	})
	server := &http.Server{Addr: ":" + strconv.Itoa(port), Handler: mux}
	server.SetKeepAlivesEnabled(false)
	return server.ListenAndServe()
}

func writeJSON(w http.ResponseWriter, value any) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(value)
}
