package main

import (
	"fmt"
	"time"
)

type recoveryRequest struct {
	delta int64
	fail  bool
}

type recoveryWorker struct {
	requests chan recoveryRequest
	replies  <-chan int64
	failed   <-chan bool
}

func startRecoveryWorker() recoveryWorker {
	requests := make(chan recoveryRequest, 1)
	replies := make(chan int64, 1)
	failed := make(chan bool, 1)
	go func() {
		defer close(failed)
		var counter int64
		for request := range requests {
			if request.fail {
				failed <- true
				return
			}
			counter += request.delta
			replies <- counter
		}
	}()
	return recoveryWorker{requests: requests, replies: replies, failed: failed}
}

func (worker recoveryWorker) add(delta int64) int64 {
	worker.requests <- recoveryRequest{delta: delta}
	return <-worker.replies
}

func runRecovery() (result, error) {
	worker := startRecoveryWorker()
	defer close(worker.requests)
	sibling := startRecoveryWorker()
	defer close(sibling.requests)
	if worker.add(1) != 1 || sibling.add(2) != 2 {
		return result{}, fmt.Errorf("recovery workers returned unexpected initial states")
	}

	replacements := make(chan recoveryWorker, 1)
	ready := make(chan struct{})
	go func() {
		current := worker
		close(ready)
		for <-current.failed {
			current = startRecoveryWorker()
			replacements <- current
		}
	}()
	<-ready
	started := time.Now()
	worker.requests <- recoveryRequest{fail: true}
	replacement := <-replacements
	state := replacement.add(0)
	elapsed := time.Since(started).Nanoseconds()
	defer close(replacement.requests)
	if state != 0 {
		return result{}, fmt.Errorf("replacement state: got %d, want 0", state)
	}
	if state := sibling.add(0); state != 2 {
		return result{}, fmt.Errorf("sibling state: got %d, want 2", state)
	}
	return result{Ops: 1, ElapsedNS: elapsed, Checksum: 1}, nil
}
