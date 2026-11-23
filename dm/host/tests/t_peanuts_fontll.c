// PEANUTS: hle_fontll.c's font engines used by one thread at a time, as a Horizon core keeps them, run on a Linux PC with real threads. hle_thread.c and hle_fontll.c
// are built as they are, as if for Android (as t_peanuts_stopworld.c does; the same stand-ins for what differs on a PC). A fake engine stands for the SDK's: SetFont
// writes 0 to the engine's font and then the new one, as FSS_set_font does (TYPESET_clear, then TYPESET_init), and GetAdvance reads the font, works a while and reads
// it again, as map_char and FSS_get_advance do (the reads that crashed in Tomodachi Life: Living the Dream). Its result is the font it measured with.
//   1. the race: two threads pinned to core 0 (priority 44 and 22) call the fake engine straight, as the host did: a call sees the font 0 or changed under it, or two
//      calls run at once (the check that this test sees the race at all);
//   2. the same through the take-overs: no call beside another, no torn font, and every SetFont + GetAdvance of the higher-priority thread measures its own font
//      (the lower-priority one waits until it blocks: an SVC); the lower-priority one gets its turns;
//   3. a higher-priority thread takes the engine between two calls of a lower-priority one that does not block: it does not wait for that one's SVC;
//   4. a lower-priority thread waits until the owner blocks: its call ends after the owner's SVC, not before;
//   5. a thread pinned to another core, and one whose mask has three cores, wait only for the call in progress, not for the owner's SVC;
//   6. a native HLE wait gives the engine back (a waiter goes on before the owner leaves it), and so does the owner's exit;
//   7. Finalize frees the engine's slot; an engine at the same address takes one anew.
// Build and run: make GAME=<id> peanuts-fontll (dm/host/tests/Makefile, part of peanuts-host); ./t_peanuts_fontll
#define _GNU_SOURCE
#include "../dm.h"
#include <assert.h>
#include <signal.h>
#include <stdatomic.h>
#include <ucontext.h>
#include <unistd.h>
static __uint128_t g_no_v[32];
#undef PLAT_UC_X
#undef PLAT_UC_FP
#undef PLAT_UC_LR
#undef PLAT_UC_SP
#undef PLAT_UC_PC
#undef PLAT_UC_CPSR
#undef PLAT_UC_V
#undef PLAT_UC_FPSR
#undef PLAT_UC_FPCR
#define PLAT_UC_X(uc, i) ((uint64_t)((uc) != NULL) * 0 + (uint64_t)(i) * 0)
#define PLAT_UC_FP(uc) 0
#define PLAT_UC_LR(uc) 0
#define PLAT_UC_SP(uc) ((uint64_t)(uc)->uc_mcontext.gregs[REG_RSP])
#define PLAT_UC_PC(uc) ((uint64_t)(uc)->uc_mcontext.gregs[REG_RIP])
#define PLAT_UC_CPSR(uc) 0
#define PLAT_UC_V(uc) g_no_v
#define PLAT_UC_FPSR(uc) 0
#define PLAT_UC_FPCR(uc) 0
#undef dm_slot_get
#define dm_slot_get(s) ((void *)0)
#include "../hle_thread.c"
#include "../hle_fontll.c"

// ---- the host services hle_thread.c and hle_fontll.c take from other files ----
struct dm_flags g_flag;
uint64_t g_t0_ns;
void crash_thread_init(void) {}
void guest_thread_init(void) {}
void guest_thread_set_id(uint64_t id) { (void)id; }
void guest_gate_stack(int own_stack) { (void)own_stack; }
uint64_t now_ns(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec; }
uint64_t guest_ns(void) { return now_ns(); }
int dm_detclock_enabled(void) { return 0; }
__thread _Atomic int dm_host_work;
pthread_key_t g_key18;
#include <sys/uio.h>
int plat_mem_read(uint64_t addr, void *out, size_t len) {
    struct iovec l = {out, len}, r = {(void *)(uintptr_t)addr, len}; return process_vm_readv(getpid(), &l, 1, &r, 1, 0) == (ssize_t)len ? 0 : -1;
}
plat_thread_t plat_thread_of(pthread_t t) { (void)t; return 0; }
plat_thread_t plat_thread_self(void) { return 0; }
void plat_thread_set_name(const char *name) { (void)name; }
int plat_thread_suspend(plat_thread_t t) { (void)t; return -1; }
int plat_thread_get_regs(plat_thread_t t, plat_regs_t *r) { (void)t; (void)r; return -1; }
void plat_thread_resume(plat_thread_t t) { (void)t; }
int plat_thread_run_state(plat_thread_t t) { (void)t; return -1; }
int64_t plat_thread_cpu_us(plat_thread_t t) { (void)t; return 0; }
int plat_threads(plat_thread_t **list, unsigned *n) { (void)list; (void)n; return -1; }
void plat_threads_release(plat_thread_t *list, unsigned n) { (void)list; (void)n; }
int plat_qos_set_self(plat_qos_t q) { (void)q; return 0; }
plat_qos_t plat_qos_of(pthread_t t) { (void)t; return PLAT_QOS_USER_INITIATED; }
void *plat_qos_override_start(pthread_t t, plat_qos_t q) { (void)t; (void)q; return NULL; }
void plat_qos_override_end(void *o) { (void)o; }
size_t plat_page_size(void) { return 4096; }
module_t *module_of(const void *p) { (void)p; return NULL; }
void *(*dm_fontll_enter)(void *engine, const char *what);   // hle_sdk16.c's (its Initialize and LoadFont take-overs are not part of this test)
void (*dm_fontll_leave)(void *slot);

// ---- the fake engine ----
typedef struct { _Atomic(const char *) font; _Atomic int inside; } fake_eng_t;
static _Atomic int g_beside, g_torn;   // calls that ran beside another one; GetAdvance calls that saw the font 0 or changed under them
static _Atomic int g_long_ms;          // GetAdvance works this long (cases 5, 6), else 20 µs
static void spin_ns(uint64_t ns) { uint64_t end = now_ns() + ns; while (now_ns() < end) {} }
static void enter_fake(fake_eng_t *e) { if (atomic_fetch_add(&e->inside, 1)) atomic_fetch_add(&g_beside, 1); }
static uint64_t fake_SetFont(void *self, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7) {
    (void)a2; (void)a3; (void)a4; (void)a5; (void)a6; (void)a7;
    fake_eng_t *e = self; enter_fake(e);
    atomic_store(&e->font, NULL); spin_ns(5000); atomic_store(&e->font, (const char *)(uintptr_t)a1);
    atomic_fetch_sub(&e->inside, 1); return 0;
}
static uint64_t fake_GetAdvance(void *self, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7) {
    (void)a1; (void)a2; (void)a3; (void)a4; (void)a5; (void)a6; (void)a7;
    fake_eng_t *e = self; enter_fake(e);
    const char *f = atomic_load(&e->font); int ms = atomic_load(&g_long_ms);
    spin_ns(ms ? ms * 1000000ull : 20000);
    if (!f || f != atomic_load(&e->font)) atomic_fetch_add(&g_torn, 1);
    atomic_fetch_sub(&e->inside, 1); return (uint64_t)(uintptr_t)f;
}
static uint64_t fake_Finalize(void *self, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7) {
    (void)self; (void)a1; (void)a2; (void)a3; (void)a4; (void)a5; (void)a6; (void)a7; return 0;
}
void *guest_sym(const char *name) {
    if (!strcmp(name, "_ZN2nn6fontll18ScalableFontEngine7SetFontEPc")) return (void *)fake_SetFont;
    if (!strcmp(name, "_ZN2nn6fontll18ScalableFontEngine10GetAdvanceEPsS2_PiS3_jt")) return (void *)fake_GetAdvance;
    if (!strcmp(name, "_ZN2nn6fontll18ScalableFontEngine8FinalizeEv")) return (void *)fake_Finalize;
    return NULL;
}
// The game's calls: straight into the engine (the host before) or through the take-overs
static int g_straight;
static void set_font(fake_eng_t *e, const char *f) { if (g_straight) fake_SetFont(e, (uintptr_t)f, 0, 0, 0, 0, 0, 0); else h_fontll_SetFont(e, (uintptr_t)f, 0, 0, 0, 0, 0, 0); }
static const char *advance(fake_eng_t *e) { return (const char *)(uintptr_t)(g_straight ? fake_GetAdvance(e, 0, 0, 0, 0, 0, 0, 0) : h_fontll_GetAdvance(e, 0, 0, 0, 0, 0, 0, 0)); }

// ---- game threads ----
void dm_thread_enter(uint64_t entry, uint64_t arg, uint64_t sp) {   // the assembly switch to a guest stack, as a C call; the end is the game's ExitThread, through the gate
    (void)sp; ((void (*)(uint64_t))entry)(arg);
    uint64_t r[100] = {0}; thr_svc_enter(r, 0); svc_thread(0x0A, r); abort();
}
static void g_svc_sleep(int64_t ns) { uint64_t r[100] = {(uint64_t)ns}; thr_svc_enter(r, 0); svc_thread(0x0B, r); thr_svc_leave(); }   // an SVC through the gate's entry and exit
typedef struct { fake_eng_t *e; const char *font; _Atomic int quit, phase; _Atomic unsigned seq, wrong; _Atomic uint64_t t_took, t_svc, t_done; } job_t;
static uint32_t start(void (*fn)(uint64_t), job_t *j, int prio, int core, uint64_t mask) {
    uint64_t r[32] = {0, (uint64_t)(uintptr_t)fn, (uint64_t)(uintptr_t)j, 0, (uint64_t)prio, (uint64_t)core};
    assert(svc_thread(0x08, r) && r[0] == 0);
    uint32_t h = (uint32_t)r[1];
    if (mask) { uint64_t m[32] = {h, (uint64_t)core, mask}; assert(svc_thread(0x0F, m) && m[0] == 0); }
    r[0] = h; assert(svc_thread(0x09, r) && r[0] == 0);
    return h;
}
// Cases 1 and 2: SetFont, game code (nn::font's SetScale and GetError come there), then GetAdvance, over and over; an SVC (a short sleep) after every fourth
static void t_seq(uint64_t arg) {
    job_t *j = (job_t *)arg; unsigned n = 0;
    while (!atomic_load(&j->quit)) {
        set_font(j->e, j->font);
        spin_ns(10000);
        if (advance(j->e) != j->font) atomic_fetch_add(&j->wrong, 1);
        atomic_fetch_add(&j->seq, 1);
        if (++n % 4 == 0) g_svc_sleep(30000);
    }
}
// Cases 3 to 6: take the engine with one call, then work in game code without an SVC until phase 1 is set from outside, then an SVC
static void t_hold(uint64_t arg) {
    job_t *j = (job_t *)arg;
    advance(j->e); atomic_store(&j->t_took, now_ns());
    while (atomic_load(&j->phase) == 0) {}
    atomic_store(&j->t_svc, now_ns()); g_svc_sleep(1000000);
    while (!atomic_load(&j->quit)) g_svc_sleep(1000000);
}
// Case 6: take the engine, then a native HLE wait until phase 1, then work without an SVC until quit
static void t_native(uint64_t arg) {
    job_t *j = (job_t *)arg;
    advance(j->e); atomic_store(&j->t_took, now_ns());
    dm_thread_native_wait(1);
    while (atomic_load(&j->phase) == 0) usleep(200);
    dm_thread_native_wait(0); atomic_store(&j->t_svc, now_ns());
    while (!atomic_load(&j->quit)) {}
}
static void t_exit(uint64_t arg) { job_t *j = (job_t *)arg; advance(j->e); atomic_store(&j->t_took, now_ns()); while (atomic_load(&j->phase) == 0) {} }
// One call into the engine; t_done when it returned
static void t_one(uint64_t arg) {
    job_t *j = (job_t *)arg;
    advance(j->e); atomic_store(&j->t_done, now_ns());
    while (!atomic_load(&j->quit)) g_svc_sleep(1000000);
}

static int g_fail;
static void check(int ok, const char *what) { printf("%s %s\n", ok ? "ok  " : "FAIL", what); g_fail += !ok; fflush(stdout); }
static void on_alarm(int sig) { (void)sig; static const char m[] = "FAIL the test hung (a font engine that never came free)\n"; if (write(1, m, sizeof m - 1)) {} _exit(99); }
static void until_set(_Atomic uint64_t *t) { while (!atomic_load(t)) usleep(100); }
static void reset(void) { atomic_store(&g_beside, 0); atomic_store(&g_torn, 0); }
static const char FONT_A[] = "A", FONT_B[] = "B";

int main(void) {
    signal(SIGALRM, on_alarm); alarm(60);
    setenv("DM_PREEMPT", "0", 1);
    svc_thread_main_init();
    // The test's own thread on core 1: on core 0 its StartThread of a higher-priority thread of core 0 would wait until that one blocks (a same-core handoff),
    //   and the owners below work without blocking until this thread tells them to stop
    { uint64_t r[32] = {H_CUR_THREAD, 1, 2}; assert(svc_thread(0x0F, r) && r[0] == 0); }

    // 1 and 2: the main thread's and 'Scene Prepare Thread''s priorities, both pinned to core 0, laying out text on one engine for 300 ms
    for (int pass = 0; pass < 2; pass++) {
        static fake_eng_t e1[2]; fake_eng_t *e = &e1[pass];
        g_straight = pass == 0; reset();
        job_t lo = {.e = e, .font = FONT_A}, hi = {.e = e, .font = FONT_B};
        start(t_seq, &lo, 44, 0, 0); start(t_seq, &hi, 22, 0, 0);
        usleep(300000);
        atomic_store(&lo.quit, 1); atomic_store(&hi.quit, 1); usleep(20000);
        printf("     %s: %u + %u sequences, %d calls beside another, %d torn, %u + %u measured with the other's font\n", pass ? "through the take-overs" : "straight",
               atomic_load(&lo.seq), atomic_load(&hi.seq), atomic_load(&g_beside), atomic_load(&g_torn), atomic_load(&lo.wrong), atomic_load(&hi.wrong));
        if (!pass) check(atomic_load(&g_beside) > 0 && atomic_load(&g_torn) > 0, "1 straight into the engine, two threads of core 0 run in it at once and see a torn font");
        else {
            check(!atomic_load(&g_beside) && !atomic_load(&g_torn), "2 through the take-overs no call runs beside another, no font is torn");
            check(!atomic_load(&hi.wrong), "2 the higher-priority thread's SetFont + GetAdvance always measure its own font");
            check(atomic_load(&lo.seq) > 0 && atomic_load(&hi.seq) > 0, "2 both threads get their turns");
        }
    }
    g_straight = 0;

    // 3: a lower-priority owner works on without an SVC for 300 ms; a higher-priority thread of its core takes the engine between two calls
    {
        static fake_eng_t e; job_t own = {.e = &e}, hi = {.e = &e};
        start(t_hold, &own, 44, 0, 0); until_set(&own.t_took);
        uint64_t t0 = now_ns(); start(t_one, &hi, 22, 0, 0); until_set(&hi.t_done);
        uint64_t waited = atomic_load(&hi.t_done) - t0;
        atomic_store(&own.phase, 1); atomic_store(&own.quit, 1); atomic_store(&hi.quit, 1);
        printf("     the higher-priority call returned after %.1f ms\n", waited / 1e6);
        check(waited < 100000000ull, "3 a higher-priority thread takes the engine between two calls of a lower-priority one that does not block");
    }
    // 4: a higher-priority owner works on without an SVC for 150 ms; a lower-priority thread of its core waits until the owner's SVC
    {
        static fake_eng_t e; job_t own = {.e = &e}, lo = {.e = &e};
        start(t_hold, &own, 22, 0, 0); until_set(&own.t_took);
        start(t_one, &lo, 44, 0, 0); usleep(150000);
        int early = atomic_load(&lo.t_done) != 0;
        atomic_store(&own.phase, 1); until_set(&lo.t_done);
        check(!early && atomic_load(&lo.t_done) >= atomic_load(&own.t_svc), "4 a lower-priority thread of the owner's core waits until the owner blocks (its SVC)");
        atomic_store(&own.quit, 1); atomic_store(&lo.quit, 1);
    }
    // 5: the same owner; a thread pinned to core 1 and one whose mask has cores 0 to 2 wait only for the call in progress (a GetAdvance of 50 ms)
    {
        static fake_eng_t e; job_t own = {.e = &e}, c1 = {.e = &e}, any = {.e = &e};
        start(t_hold, &own, 22, 0, 0); until_set(&own.t_took);
        uint64_t t0 = now_ns(); start(t_one, &c1, 44, 1, 0); until_set(&c1.t_done);
        uint64_t w1 = atomic_load(&c1.t_done) - t0;
        t0 = now_ns(); start(t_one, &any, 44, 0, 7); until_set(&any.t_done);
        uint64_t w7 = atomic_load(&any.t_done) - t0;
        int held = atomic_load(&own.phase) == 0;
        atomic_store(&own.phase, 1); atomic_store(&own.quit, 1); atomic_store(&c1.quit, 1); atomic_store(&any.quit, 1);
        printf("     core 1: %.1f ms, mask 0x7: %.1f ms\n", w1 / 1e6, w7 / 1e6);
        check(held && w1 < 100000000ull && w7 < 100000000ull, "5 a thread of another core, and one that may run on three, do not wait for the owner's SVC");
        reset(); atomic_store(&g_long_ms, 50);
        static fake_eng_t e2; job_t in = {.e = &e2}, o1 = {.e = &e2};
        start(t_one, &in, 22, 0, 0); while (!atomic_load(&e2.inside)) usleep(100);   // in its 50 ms call
        start(t_one, &o1, 44, 1, 0); until_set(&o1.t_done); until_set(&in.t_done);
        check(!atomic_load(&g_beside) && atomic_load(&o1.t_done) >= atomic_load(&in.t_done), "5 ... but they wait for a call in progress");
        atomic_store(&g_long_ms, 0); atomic_store(&in.quit, 1); atomic_store(&o1.quit, 1);
    }
    // 6: an owner of the higher priority in a native HLE wait, then one that exits
    {
        static fake_eng_t e; job_t own = {.e = &e}, lo = {.e = &e};
        start(t_native, &own, 22, 0, 0); until_set(&own.t_took); usleep(5000);
        start(t_one, &lo, 44, 0, 0); usleep(100000);
        int during = atomic_load(&lo.t_done) != 0;
        atomic_store(&own.phase, 1); usleep(5000); atomic_store(&own.quit, 1); atomic_store(&lo.quit, 1);
        check(during, "6 a native HLE wait gives the engine back: a lower-priority thread of the core goes on before the owner leaves it");
        static fake_eng_t e2; job_t ex = {.e = &e2}, lo2 = {.e = &e2};
        start(t_exit, &ex, 22, 0, 0); until_set(&ex.t_took);
        start(t_one, &lo2, 44, 0, 0); usleep(50000);
        int early = atomic_load(&lo2.t_done) != 0;
        atomic_store(&ex.phase, 1); until_set(&lo2.t_done); atomic_store(&lo2.quit, 1);
        check(!early, "6 ... and the owner's exit does: the waiter went on only after it");
    }
    // 7: Finalize frees the slot
    {
        static fake_eng_t e;
        h_fontll_GetAdvance(&e, 0, 0, 0, 0, 0, 0, 0);
        int had = 0, has = 0;
        for (int i = 0; i < font_eng_n; i++) had |= font_eng[i].self == &e;
        h_fontll_Finalize(&e, 0, 0, 0, 0, 0, 0, 0);
        for (int i = 0; i < font_eng_n; i++) has |= font_eng[i].self == &e;
        h_fontll_GetAdvance(&e, 0, 0, 0, 0, 0, 0, 0);
        int again = 0; for (int i = 0; i < font_eng_n; i++) again |= font_eng[i].self == &e;
        check(had && !has && again, "7 Finalize frees the engine's slot, and the engine takes one again on its next call");
    }
    printf("%s\n", g_fail ? "FAILED" : "all passed");
    return g_fail != 0;
}
