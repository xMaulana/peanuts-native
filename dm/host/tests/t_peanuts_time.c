// PEANUTS: the time service's clocks on a phone that sleeps and restarts (GitHub issue #329; hle_time.c built as for Android, on a Linux PC). The host's real-time,
// monotonic and guest clocks are stand-ins the test moves; each "run" of the game is a child process, so hle_time.c's per-run state (the custom time's offset,
// the user clock context) starts fresh as in a new game process, and snapshots pass between runs through shared memory as a save carries them.
//   1. a phone that slept between two runs: no user clock change, the span is the real time that passed (the bug: the sleep came out as the user moving the clock);
//   2. a phone restarted between two runs (the monotonic clock starts again at 0): the same;
//   3. a sleep while the game runs: the context is made again (Unity notices), its offset stays, no user clock change;
//   4. a custom time (DM_TIME_OFFSET, DM_TIME) is a user clock change, the same in every snapshot of its run, and its clock runs with the host's;
//   5. a snapshot of the old monotonic steady clock (source "dmhost-steady-01"): a span to it is unknown, no user clock change;
//   6. a snapshot's user time is its context's offset plus its steady time point (one reading); DM_FIXED_TIME stays fixed;
//   7. two runs whose clocks sit at other points within the second: no user clock change (two readings came out a second apart, which the game also warns about);
//   8. the phone's clock corrected back (or forward) while the game runs: no negative span, no user clock change (the follow-up of PR #336: the real-time clock
//      is read once a run and advanced by the boot-time clock, which counts sleep and never goes back), nor against the next run, which starts on the corrected clock;
//   9. the first reads of the user clock from eight threads at once all carry the custom time's offset.
// make GAME=<id> peanuts-time (part of peanuts-host). No game files.
#include "../hle.h"
#undef HLE_FORCE_BIG
#define HLE_FORCE_BIG HLE_FORCE   // the big-stack entry is an arm64 stub; on the PC the test calls the implementations directly
#include "../hle_time.c"
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

// The host's clocks: seconds the test sets (real time) and moves (monotonic: stops while the phone sleeps, starts again at a restart; boot time: counts sleep,
// starts again at a restart)
static int64_t g_rt = 1795251429, g_mono = 31523, g_boot = 31523, g_rt_sub, g_mono_sub;   // seconds, and nanoseconds within the second
uint64_t plat_clock_ns(clockid_t c) {
    if (c == CLOCK_REALTIME) return (uint64_t)g_rt * 1000000000ull + g_rt_sub;
    if (c == CLOCK_BOOTTIME) return (uint64_t)g_boot * 1000000000ull + g_mono_sub;
    return (uint64_t)g_mono * 1000000000ull + g_mono_sub;
}
uint64_t guest_ns(void) { return (uint64_t)g_mono * 1000000000ull + g_mono_sub; }   // the guest ticks run on the monotonic clock (hle_os.c now_ns)

static int checks, failed;
#define CHECK(c, ...) do { checks++; if (!(c)) { failed++; printf("FAIL %s:%d: ", __FILE__, __LINE__); printf(__VA_ARGS__); printf("\n"); } else { printf("ok   "); printf(__VA_ARGS__); printf("\n"); } } while (0)
static int64_t get64(const uint8_t *s, int off) { int64_t v; memcpy(&v, s + off, 8); return v; }
static void put64(uint8_t *s, int off, int64_t v) { memcpy(s + off, &v, 8); }

// One run of the game: a child process with the clocks at rt / mono and the environment env ("NAME=value", or NULL); it takes snapshot *out (adjustable, as Tomodachi Life)
static void run(int64_t rt, int64_t mono, const char *env, uint8_t *out) {
    pid_t p = fork();
    if (!p) {
        g_rt = rt; g_mono = mono; g_boot = mono;
        if (env) putenv((char *)env);
        _exit(h_time_AdjSnapshot(out) == RES_OK ? 0 : 1);
    }
    int st = 0; waitpid(p, &st, 0);
    CHECK(WIFEXITED(st) && !WEXITSTATUS(st), "a run at %lld (monotonic %lld) took its snapshot", (long long)rt, (long long)mono);
}
static void *first_read(void *out) { *(int64_t *)out = user_now(); return NULL; }
static int64_t span_s(const uint8_t *a, const uint8_t *b, int *ok) { int64_t ns = 0x5a5a; *ok = h_time_SpanBetween(&ns, a, b); return ns / 1000000000; }

int main(void) {
    setenv("TZ", "UTC", 1); tzset();
    uint8_t *snap = mmap(NULL, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    uint8_t *a = snap, *b = snap + 0x100, *c = snap + 0x200;
    int ok;

    // 1. Played, the phone slept 7 h 50 min and was used 10 min, played again: 8 h of real time, 10 min of monotonic time
    run(1795251429, 31523, NULL, a);
    run(1795251429 + 8 * 3600, 31523 + 600, NULL, b);
    CHECK(!memcmp(a + 0x98, STEADY_SOURCE, 16) && !memcmp(a + 0x10, STEADY_SOURCE, 16), "the snapshot's steady time point and its context's are of the host's steady clock");
    CHECK(h_time_UserDiff(a, b) == 0, "a sleep between two runs is no user clock change (was %lld s)", (long long)h_time_UserDiff(a, b) / 1000000000);
    CHECK(span_s(a, b, &ok) == 8 * 3600 && ok, "the span is the 8 h that passed, sleep included");
    CHECK(get64(b, 0x40) - get64(a, 0x40) == 8 * 3600, "the user clock moved 8 h");

    // 2. The phone restarted: the monotonic clock starts again near 0, the real-time clock goes on
    run(1795251429 + 9 * 3600, 40, NULL, c);
    CHECK(h_time_UserDiff(b, c) == 0, "a restart between two runs is no user clock change");
    CHECK(span_s(b, c, &ok) == 3600 && ok, "the span across the restart is the hour that passed, not negative");

    // 3. The phone slept 3 h while the game ran: the guest ticks stood still, the real-time clock went on
    {
        pid_t p = fork();
        if (!p) {
            uint8_t *x = snap + 0x300, *y = snap + 0x400;
            g_rt = 1795300000; g_mono = g_boot = 5000;
            h_time_AdjSnapshot(x);
            g_rt += 3 * 3600; g_boot += 3 * 3600;   // asleep: the monotonic clock stands still
            h_time_AdjSnapshot(y);
            _exit(0);
        }
        int st = 0; waitpid(p, &st, 0);
        uint8_t *x = snap + 0x300, *y = snap + 0x400;
        CHECK(h_time_UserDiff(x, y) == 0, "a sleep during the run is no user clock change");
        CHECK(span_s(x, y, &ok) == 3 * 3600 && ok, "the span is the 3 h of sleep");
        CHECK(get64(x, 0x08) != get64(y, 0x08) && get64(x, 0x00) == get64(y, 0x00), "the context was made again (a new steady time point, the same offset)");
    }

    // 4. A custom time is the user setting the clock: DM_TIME_OFFSET, then DM_TIME
    run(1795251429 + 10 * 3600, 900, "DM_TIME_OFFSET=7200", b);
    CHECK(h_time_UserDiff(c, b) == 7200ll * 1000000000, "DM_TIME_OFFSET=7200 is a user clock change of 2 h");
    CHECK(get64(b, 0x40) == 1795251429 + 10 * 3600 + 7200, "its user time is the host clock plus 2 h");
    run(1795251429 + 10 * 3600, 900, "DM_TIME=2026-11-22T12:00", a);
    CHECK(get64(a, 0x40) == 1795348800, "DM_TIME starts the user clock at its time (2026-11-22 12:00 UTC)");
    CHECK(h_time_UserDiff(c, a) == (1795348800ll - (1795251429 + 10 * 3600)) * 1000000000, "DM_TIME is a user clock change of its distance to the host clock");
    {
        pid_t p = fork();
        if (!p) {
            uint8_t x[0xd0], y[0xd0]; int bad = 0, o;
            putenv("DM_TIME=2026-11-22T12:00"); g_rt = 1795300000; g_mono = g_boot = 100;
            h_time_AdjSnapshot(x);
            g_rt += 5 * 3600; g_boot += 5 * 3600;   // 5 h of sleep in the same run
            h_time_AdjSnapshot(y);
            bad |= get64(y, 0x40) - get64(x, 0x40) != 5 * 3600;   // DM_TIME's clock runs with the host's, sleep included
            bad |= h_time_UserDiff(x, y) != 0;                      // and its offset stays: no new change
            int64_t ns = 0; o = h_time_SpanBetween(&ns, x, y); bad |= !o || ns != 5ll * 3600 * 1000000000;
            _exit(bad);
        }
        int st = 0; waitpid(p, &st, 0);
        CHECK(WIFEXITED(st) && !WEXITSTATUS(st), "DM_TIME's clock runs on through a sleep and is changed only once");
    }

    // 5. A save from before the fix: its steady time points are of the monotonic clock, source "dmhost-steady-01"
    memcpy(a, c, 0xd0); memcpy(a + 0x98, "dmhost-steady-01", 16); memcpy(a + 0x10, "dmhost-steady-01", 16); put64(a, 0x90, 31723); put64(a, 0x08, 31523);
    int64_t keep = 0x1234; ok = h_time_SpanBetween(&keep, a, c);
    CHECK(!ok && keep == 0x1234, "a span to the old steady clock is unknown, as after an RTC reset (no network time offline)");
    CHECK(h_time_UserDiff(a, c) == 0, "and no user clock change");

    // 6. One reading: the user time is the context's offset plus the steady time point; DM_FIXED_TIME stays fixed
    run(1795400000, 77, NULL, a);
    CHECK(get64(a, 0x40) == get64(a, 0x00) + get64(a, 0x90), "user time = context offset + steady time point");
    run(1795400000, 77, "DM_FIXED_TIME=1700000000", b);
    CHECK(get64(b, 0x40) == 1700000000, "DM_FIXED_TIME's user time");

    // 7. No sleep, the host clock 0.8 s ahead of the monotonic one within the second: read at .9 / .1 and, 100.3 s later, at .2 / .4 (the Thor's two runs said -1 s)
    {
        pid_t p = fork();
        if (!p) { g_rt = 1795400000; g_rt_sub = 900000000; g_mono = g_boot = 31523; g_mono_sub = 100000000; _exit(h_time_AdjSnapshot(a) != RES_OK); }
        int st = 0; waitpid(p, &st, 0);
        p = fork();
        if (!p) { g_rt = 1795400101; g_rt_sub = 200000000; g_mono = g_boot = 31623; g_mono_sub = 400000000; _exit(h_time_AdjSnapshot(b) != RES_OK); }
        waitpid(p, &st, 0);
        CHECK(h_time_UserDiff(a, b) == 0, "the clocks' phases within the second are no user clock change (was %lld s)", (long long)h_time_UserDiff(a, b) / 1000000000);
    }

    // 8. While the game runs, Android corrects the phone's clock: 10 minutes back after 5 s, then 20 minutes forward after 5 s more
    {
        pid_t p = fork();
        if (!p) {
            uint8_t *x = snap + 0x300, *y = snap + 0x400, *z = snap + 0x500;
            g_rt = 1795500000; g_rt_sub = 0; g_mono = g_boot = 9000; g_mono_sub = 0;
            h_time_AdjSnapshot(x);
            g_rt += 5 - 600; g_mono += 5; g_boot += 5;
            h_time_AdjSnapshot(y);
            g_rt += 5 + 1200; g_mono += 5; g_boot += 5;
            h_time_AdjSnapshot(z);
            _exit(0);
        }
        int st = 0; waitpid(p, &st, 0);
        uint8_t *x = snap + 0x300, *y = snap + 0x400, *z = snap + 0x500;
        CHECK(span_s(x, y, &ok) == 5 && ok, "the clock set 10 minutes back: the span is the 5 s that passed, not negative (was %lld s)", (long long)span_s(x, y, &ok));
        CHECK(span_s(y, z, &ok) == 5 && ok, "the clock set 20 minutes forward: the span is the 5 s that passed");
        CHECK(h_time_UserDiff(x, y) == 0 && h_time_UserDiff(y, z) == 0, "and no user clock change");
        CHECK(get64(y, 0x40) - get64(x, 0x40) == 5 && get64(z, 0x40) - get64(y, 0x40) == 5, "the user clock runs on with the steady clock within the run");
        // The game saved z; the phone restarted and the next run starts an hour after it, on the corrected clock (net 10 minutes ahead of the run's anchor)
        uint8_t *w = snap + 0x600;
        run(1795500000 + 610 + 3600, 50, NULL, w);
        CHECK(get64(w, 0x00) == 0, "the next run's context has offset 0");
        CHECK(h_time_UserDiff(z, w) == 0, "a snapshot saved after the corrections, against the next run: no user clock change (was %lld s)", (long long)h_time_UserDiff(z, w) / 1000000000);
        CHECK(span_s(z, w, &ok) == 3600 + 600 && ok, "the correction reaches the next run as time that passed (%lld s)", (long long)span_s(z, w, &ok));
    }

    // 9. The first reads of the user clock from several threads at once: every one sees the custom time's offset (the mode is never read before its offset)
    {
        pid_t p = fork();
        if (!p) {
            putenv("DM_TIME_OFFSET=3600"); g_rt = 1795600000;
            pthread_t t[8]; int64_t got[8];
            for (int i = 0; i < 8; i++) pthread_create(&t[i], NULL, first_read, &got[i]);
            int bad = 0;
            for (int i = 0; i < 8; i++) { pthread_join(t[i], NULL); bad |= got[i] != 1795600000 + 3600; }
            _exit(bad);
        }
        int st = 0; waitpid(p, &st, 0);
        CHECK(WIFEXITED(st) && !WEXITSTATUS(st), "eight threads' first reads all carry DM_TIME_OFFSET");
    }

    printf("peanuts time: %d checks, %d failed\n", checks, failed);
    return failed != 0;
}
