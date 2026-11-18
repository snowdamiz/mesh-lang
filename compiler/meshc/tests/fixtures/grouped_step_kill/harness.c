// Driver for the generated start_victim function in main.mpl.
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t word;
extern void mesh_rt_init(void), mesh_rt_init_actor(uint32_t), mesh_rt_run_scheduler(void);
extern word mesh_actor_self(void), Main__start_victim(word parent, word turns);
extern int64_t mesh_actor_send(word, const void *, word);
extern void mesh_actor_exit(word, uint8_t), mesh_actor_link(word);
typedef void (*callback)(const void *, const uint8_t *);
extern void mesh_actor_set_terminate(word, callback);
extern word mesh_actor_spawn_grouped(void (*)(const void *), const void *, word,
                                     const uint32_t *, callback);
extern word mesh_process_monitor(word, const void *, word, const uint32_t *);
extern void *mesh_actor_receive_scalar(int64_t, word, word *);

enum { KILLED = 2, LINKED = 3, VICTIM_DOWN = 90001, PEER_DOWN = 90002 };
enum { FAIR_TURNS = 12000, PER_SENDER = 64 };
static word victim;
static unsigned pause_at;
static int fairness;
static atomic_uint replies, consumed, callback_count, reason, witness_at;
static atomic_bool released, peer_go, peer_ready, peer_done;

static void check(int ok, const char *message) {
    if (!ok) {
        fprintf(stderr, "%s (replies=%u, callbacks=%u, reason=%u)\n",
                message, atomic_load(&replies), atomic_load(&callback_count), atomic_load(&reason));
        exit(1);
    }
}

static void wait_flag(atomic_bool *flag) {
    while (!atomic_load(flag)) sched_yield();
}

static void send_delta(word to) {
    word delta = 1;
    check(mesh_actor_send(to, &delta, sizeof(delta)) == 0, "send failed");
}

static void witness(const void *unused) {
    (void)unused;
    atomic_store(&witness_at, atomic_load(&replies));
}

// Only sends from the generated module go through this hook. Keep the real
// delivery, return status and generated tail transfer on both lowering paths.
int64_t observed_send(word to, const void *msg, word size) {
    int64_t status = mesh_actor_send(to, msg, size);
    check(status == 0, "fixture reply was rejected");
    unsigned n = atomic_fetch_add(&replies, 1) + 1;
    if (pause_at > 1 && n == 1) {
        // A grouped child inherits this worker. Its first instruction proves
        // the victim relinquished the worker before the host starts turn two.
        mesh_actor_spawn_grouped(witness, NULL, 0, NULL, NULL);
    }
    if (n == pause_at) wait_flag(&released);
    if (fairness && n < FAIR_TURNS) {
        // Keep the victim runnable while bounding the host's mailbox. This
        // OS-thread wait cannot run the witness pinned to this worker.
        while (n - atomic_load(&consumed) >= 256) sched_yield();
        send_delta(mesh_actor_self());
    }
    return status;
}

static void on_exit_reason(const void *state, const uint8_t *tag) {
    (void)state;
    atomic_store(&reason, *tag);
    atomic_fetch_add(&callback_count, 1);
}

static void start(unsigned turns) {
    victim = Main__start_victim(mesh_actor_self(), turns);
    mesh_actor_set_terminate(victim, on_exit_reason);
    word down = VICTIM_DOWN;
    mesh_process_monitor(victim, &down, sizeof(down), NULL);
}

static word receive(void) {
    word scratch;
    word *value = mesh_actor_receive_scalar(5000, sizeof(word), &scratch);
    check(value != NULL, "missing mailbox delivery");
    return *value;
}

static void expect_replies(unsigned count) {
    static word a = 1, b = 3;
    for (unsigned i = 0; i < count; ++i) {
        check(receive() == a, "unexpected rotate_words frame or mailbox order");
        atomic_fetch_add(&consumed, 1);
        word next = b + 1;
        b = a;
        a = next;
    }
}

static void expect_down(unsigned tag, unsigned count) {
    check(receive() == VICTIM_DOWN, "extra victim reply or missing DOWN");
    check(atomic_load(&callback_count) == 1 && atomic_load(&reason) == tag,
          "incorrect terminate callback at DOWN");
    mesh_rt_run_scheduler();
    check(atomic_load(&callback_count) == 1 && atomic_load(&replies) == count,
          "incorrect final effect count");
}

static void killed_neighbor(const void *unused) {
    (void)unused;
    mesh_actor_link(victim);
    atomic_store(&peer_ready, 1);
    wait_flag(&peer_go);
    mesh_actor_exit(mesh_actor_self(), KILLED);
}

static void kill_during_reply(int linked, unsigned turn) {
    pause_at = turn;
    unsigned queued = turn == 1 ? 1 : 3;
    // Exercise return with the first kill and an empty receive with the later kill.
    start(turn + queued + (turn > 1));
    send_delta(victim);
    if (turn > 1) {
        while (!atomic_load(&witness_at)) sched_yield();
        for (unsigned i = 1; i < turn; ++i) send_delta(victim);
    }
    expect_replies(turn);
    while (atomic_load(&replies) < turn) sched_yield();
    // Queue the remaining messages while the handler is active.
    for (unsigned i = 0; i < queued; ++i) send_delta(victim);
    if (linked) {
        word peer = mesh_actor_spawn_grouped(killed_neighbor, NULL, 0, NULL, NULL);
        wait_flag(&peer_ready);
        word down = PEER_DOWN;
        mesh_process_monitor(peer, &down, sizeof(down), NULL);
        atomic_store(&peer_go, 1);
        // The peer's DOWN is emitted after propagation to its links.
        check(receive() == PEER_DOWN, "missing linked exit");
    } else {
        mesh_actor_exit(victim, KILLED);
    }
    atomic_store(&released, 1);
    // Grouped coroutines observe forced exit at suspension or return.
    // All queued deltas fit in this quantum on either path.
    expect_replies(queued);
    expect_down(linked ? LINKED : KILLED, turn + queued);
}

static void occupy_worker(const void *unused) {
    (void)unused;
    atomic_store(&peer_ready, 1);
    wait_flag(&released);
}

static void prestart(void) {
    mesh_actor_spawn_grouped(occupy_worker, NULL, 0, NULL, NULL);
    wait_flag(&peer_ready);
    start(2);
    send_delta(victim);
    mesh_actor_exit(victim, KILLED);
    atomic_store(&released, 1);
    expect_down(KILLED, 0);
}

static void fair(void) {
    fairness = 1;
    pause_at = 1;
    start(FAIR_TURNS);
    send_delta(victim);
    while (!atomic_load(&replies)) sched_yield();
    mesh_actor_spawn_grouped(witness, NULL, 0, NULL, NULL);
    atomic_store(&released, 1);
    expect_replies(FAIR_TURNS);
    expect_down(0, FAIR_TURNS);
    check(atomic_load(&witness_at) > 0 && atomic_load(&witness_at) < FAIR_TURNS,
          "queued peer failed to run before victim drained");
}

static void send_batch(word base) {
    for (unsigned i = 1; i <= PER_SENDER; ++i) {
        word delta = base + i;
        check(mesh_actor_send(victim, &delta, sizeof(delta)) == 0, "producer send failed");
    }
}

static void produce(const void *unused) {
    (void)unused;
    atomic_store(&peer_ready, 1);
    wait_flag(&peer_go);
    send_batch(2000);
    atomic_store(&peer_done, 1);
}

static void concurrent(void) {
    pause_at = 1;
    unsigned count = 2 + 2 * PER_SENDER;
    start(count);
    send_delta(victim);
    while (!atomic_load(&replies)) sched_yield();
    mesh_actor_spawn_grouped(produce, NULL, 0, NULL, NULL);
    wait_flag(&peer_ready);
    atomic_store(&peer_go, 1);
    send_batch(1000);
    wait_flag(&peer_done);
    send_delta(victim);  // Flush the last producer delta into a visible frame.
    atomic_store(&released, 1);

    word a = receive(), b = receive();
    check(a == 1 && b == 4, "unexpected opening frames");
    word host_next = 1001, peer_next = 2001;
    for (unsigned i = 0; i < 2 * PER_SENDER; ++i) {
        word next = receive();
        word delta = next - a;
        if (delta == host_next) ++host_next;
        else if (delta == peer_next) ++peer_next;
        else check(0, "lost, duplicate or reordered producer message");
        a = b;
        b = next;
    }
    check(host_next == 1001 + PER_SENDER && peer_next == 2001 + PER_SENDER,
          "incomplete producer batch");
    expect_down(0, count);
}

int main(int argc, char **argv) {
    check(argc >= 2, "missing case name");
    int linked = strcmp(argv[1], "linked") == 0;
    int producers = strcmp(argv[1], "concurrent") == 0;
    mesh_rt_init();
    mesh_rt_init_actor(linked || producers ? 2 : 1);
    if (linked || strcmp(argv[1], "direct") == 0) {
        check(argc == 3, "missing kill turn");
        kill_during_reply(linked, (unsigned)strtoul(argv[2], NULL, 10));
    } else if (producers) concurrent();
    else if (strcmp(argv[1], "prestart") == 0) prestart();
    else if (strcmp(argv[1], "fair") == 0) fair();
    else check(0, "unknown case");
    return 0;
}
