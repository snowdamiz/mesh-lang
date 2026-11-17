package main

import (
	"fmt"
	"sync"
	"time"
)

const sampleInterval = 64

type actorResult struct {
	result
	RTTNS []int64 `json:"rtt_ns"`
}

type callerResult struct {
	checksum int64
	finished time.Time
	samples  []int64
	err      error
}

func runActor(pairs, iters, warmup int) (actorResult, error) {
	if warmup != 0 {
		if _, err := actorRound(pairs, warmup, false); err != nil {
			return actorResult{}, err
		}
	}
	return actorRound(pairs, iters, true)
}

func actorRound(pairs, iters int, timed bool) (actorResult, error) {
	start := make(chan struct{})
	callers := make([]callerResult, pairs)
	var ready, done sync.WaitGroup
	ready.Add(2 * pairs)
	for i := range callers {
		requests := make(chan int64, 1)
		replies := make(chan int64, 1)
		if timed && iters > 0 {
			callers[i].samples = make([]int64, 0, (iters-1)/sampleInterval+1)
		}
		done.Go(func() {
			ready.Done()
			var counter int64
			for value := range requests {
				counter += value
				replies <- counter
			}
		})
		done.Go(func() {
			defer close(requests)
			ready.Done()
			<-start
			callers[i] = callWorker(requests, replies, iters, timed, callers[i].samples)
		})
	}
	ready.Wait()
	var started time.Time
	if timed {
		started = time.Now()
	}
	close(start)
	done.Wait()

	stats := actorResult{
		result: result{Ops: int64(pairs) * int64(iters)},
		RTTNS:  make([]int64, 0),
	}
	finished := started
	for _, caller := range callers {
		if caller.err != nil {
			return actorResult{}, caller.err
		}
		stats.Checksum += caller.checksum
		stats.RTTNS = append(stats.RTTNS, caller.samples...)
		if caller.finished.After(finished) {
			finished = caller.finished
		}
	}
	stats.ElapsedNS = finished.Sub(started).Nanoseconds()
	return stats, nil
}

func callWorker(requests chan<- int64, replies <-chan int64, count int, timed bool, samples []int64) callerResult {
	var expected, reply int64
	var finished time.Time
	for i := range count {
		value := int64(i) + 1
		expected += value
		sample := timed && i%sampleInterval == 0
		var sent, received time.Time
		if sample {
			sent = time.Now()
		}
		requests <- value
		reply = <-replies
		if sample || (timed && i == count-1) {
			received = time.Now()
		}
		if reply != expected {
			return callerResult{err: fmt.Errorf("actor reply for iteration %d: got %d, want %d", value, reply, expected)}
		}
		if sample {
			samples = append(samples, received.Sub(sent).Nanoseconds())
		}
		finished = received
	}
	return callerResult{checksum: reply, finished: finished, samples: samples}
}
