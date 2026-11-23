// PEANUTS begin: nn::fontll::ScalableFontEngine used by one thread at a time, as a Horizon core keeps it (the whole file is Peanuts'; docs/reference/threads.md,
//   "Two threads of one core in a font engine").
// An engine (the object, 0xb8 bytes on SDK 20, is the iType font state that every FSS_ function works on) takes no lock. NintendoWare Font's TextureCache, in the
//   game's main, keeps one engine per core and picks it by the caller's core number (Tomodachi Life: Living the Dream 1.0.4: main+0x693930 builds three, main+0x1d0d470
//   picks by sead's cached core number or nn::os::GetCurrentCoreNumber), so threads share an engine only when they share a core. On Horizon two threads pinned to one
//   core never run at once: the higher-priority one runs until it blocks, and the other does not run meanwhile. The host runs them side by side.
//   Tomodachi Life's job worker 'AppJobThreadPool0' (priority 44) and its 'Scene Prepare Thread' (created at 22, running at 50) are both pinned to core 0 and lay out text on engine 0. One thread's SetFont
//   (FSS_set_font sdk+0x378dc0: TYPESET_clear writes 0 to the font at +0x10, sdk+0x3ae4d8, and +0x30 / +0x38 are cleared at sdk+0x378f54) ran while the other was in
//   map_char or FSS_get_advance, which read those pointers again: CRASH sig=11 at 0x20 to 0x28 (sdk+0x384220, sdk+0x37cd34, sdk+0x37cd68, sdk+0x37cdfc), 4 boots of 8.
// So every engine call of the game's modules goes through font_enter / font_leave:
//   - a call never runs beside another call into the same engine;
//   - between two threads pinned to the same core, the engine stays with the one that used it last until that one blocks (its next SVC, a native HLE wait, its exit:
//     dm_core_excl_release), unless the other has the higher priority: that one takes it between two calls, as it preempts on Horizon. nn::font's sequences
//     (SetFont when its per-engine cache names another font, SetScale, CheckGlyphExist, GetAdvance: main+0x105ee64, main+0x105ed1c) then run on one thread's font
//     and scale, as there. Exclusion per call alone would leave a thread's CheckGlyphExist or GetAdvance working on the font the other thread set in between;
//   - any other pair (a thread that may run on other cores, a host thread) waits only for the call in progress. Such threads would be on different cores, with different
//     engines, on Horizon. The host's core number for a thread that may run on several cores is one core of its mask, so they can still share an engine here.
#include "hle.h"
#include <pthread.h>
#include <time.h>

#define FONT_ENG_SLOTS 64
typedef struct { void *self; uint64_t owner; int inside, waiters; } font_eng_t;   // owner: a guest thread's ID (dm_thread_id_self), a host thread's FONT_HOST_ID; 0 = free
static font_eng_t font_eng[FONT_ENG_SLOTS];
static int font_eng_n;
static pthread_mutex_t font_eng_mu = PTHREAD_MUTEX_INITIALIZER;   // lock order: an HLE call's own mutex, then font_eng_mu, then hle_thread.c's G
static pthread_cond_t font_eng_cv;     // made in font_attach: on Android for CLOCK_MONOTONIC, as hle_thread.c's waits (Android's wall clock is set while a game runs)
#ifdef __ANDROID__
#define FONT_CLOCK CLOCK_MONOTONIC
#else
#define FONT_CLOCK CLOCK_REALTIME      // Apple has no pthread_condattr_setclock
#endif
static unsigned font_eng_waits, font_eng_takes;   // font_eng_mu: waits and take-overs so far (the log names the first and each power of two)
static char font_who[2][160];          // font_eng_mu: the two threads those lines name, off the guest stack (a game thread's can be 8 KB, and these take-overs are not on the big stack)
static __thread int font_eng_waiting;    // this thread enters its wait in font_enter: the native wait's release must not take font_eng_mu, which it holds
#define FONT_HOST_ID(me) ((me) ? (me) : ((uint64_t)(uintptr_t)pthread_self() | 1ull << 63))
// A wait looks again this often: an owner can stop being runnable without passing an SVC or a native wait (a pause by the game, host GPU work)
#define FONT_RECHECK_NS 1000000

// Holding font_eng_mu: the engines me owns and is not inside go back
static void font_release_locked(uint64_t me) {
    int still = 0, woke = 0;
    for (int i = 0; i < font_eng_n; i++) {
        font_eng_t *e = &font_eng[i];
        if (!e->self || e->owner != me) continue;
        if (e->inside) { still = 1; continue; }   // an engine call that itself makes an SVC (an allocator's mutex) keeps its engine
        e->owner = 0; woke |= e->waiters;
    }
    dm_core_excl_held = still;
    if (woke) pthread_cond_broadcast(&font_eng_cv);
}
// dm_core_excl_release: the calling thread blocks (hle_thread.c, at an SVC and at a native HLE wait)
static void font_release_self(void) {
    if (font_eng_waiting) return;
    uint64_t me = dm_thread_id_self();
    if (!me) { dm_core_excl_held = 0; return; }
    pthread_mutex_lock(&font_eng_mu); font_release_locked(me); pthread_mutex_unlock(&font_eng_mu);
}
static font_eng_t *font_slot(void *self) {   // holding font_eng_mu; NULL when the table is full
    font_eng_t *free_slot = NULL;
    for (int i = 0; i < font_eng_n; i++) {
        if (font_eng[i].self == self) return &font_eng[i];
        if (!font_eng[i].self && !free_slot) free_slot = &font_eng[i];
    }
    if (!free_slot && font_eng_n < FONT_ENG_SLOTS) free_slot = &font_eng[font_eng_n++];
    if (free_slot) *free_slot = (font_eng_t){.self = self};
    return free_slot;
}
static font_eng_t *font_enter(void *self, const char *what) {
    uint64_t me = FONT_HOST_ID(dm_thread_id_self());
    pthread_mutex_lock(&font_eng_mu);
    font_eng_t *e = font_slot(self);
    if (!e) {
        static int said; if (!said++) LOG("[RED] more than %d font engines: engine %p is used without the exclusion of a core (hle_fontll.c)", FONT_ENG_SLOTS, self);
        pthread_mutex_unlock(&font_eng_mu); return NULL;
    }
    while (e->owner && e->owner != me) {
        int in_call = e->inside > 0;
        if (!in_call && !dm_thread_core_excl_wait(e->owner)) {   // free between two calls for this thread: it takes the engine over
            unsigned n = ++font_eng_takes;
            if (!(n & (n - 1))) {
                char *a = font_who[0], *b = font_who[1]; dm_thread_describe(me >> 63 ? 0 : me, a, sizeof font_who[0]); dm_thread_describe(e->owner >> 63 ? 0 : e->owner, b, sizeof font_who[1]);
                LOG("font engine %p: %s takes it in %s from %s between two of its calls (take-over %u)", self, a, what, b, n);
            }
            break;
        }
        unsigned n = ++font_eng_waits;
        if (!(n & (n - 1))) {
            char *a = font_who[0], *b = font_who[1]; dm_thread_describe(me >> 63 ? 0 : me, a, sizeof font_who[0]); dm_thread_describe(e->owner >> 63 ? 0 : e->owner, b, sizeof font_who[1]);
            LOG("font engine %p: %s waits in %s for %s, %s (wait %u; two threads of one core in one engine, which Horizon's core keeps apart)", self, a, what, b,
                in_call ? "inside a call" : "until it blocks", n);
        }
        font_release_locked(me);   // a wait blocks this thread: the engines it holds go back, as its core would
        e->waiters++;
        struct timespec ts; clock_gettime(FONT_CLOCK, &ts);
        ts.tv_nsec += FONT_RECHECK_NS; if (ts.tv_nsec >= 1000000000) { ts.tv_sec++; ts.tv_nsec -= 1000000000; }
        font_eng_waiting = 1; dm_thread_native_wait(1); font_eng_waiting = 0;
        pthread_cond_timedwait(&font_eng_cv, &font_eng_mu, &ts);
        // Back to runnable without font_eng_mu: a thread paused meanwhile (the game's SetThreadActivity, a stop of all threads) waits in there until it is resumed
        pthread_mutex_unlock(&font_eng_mu); dm_thread_native_wait(0); pthread_mutex_lock(&font_eng_mu);
        e->waiters--;
    }
    e->owner = me; e->inside++;
    if (!(me >> 63)) dm_core_excl_held = 1;
    pthread_mutex_unlock(&font_eng_mu);
    return e;
}
static void font_leave(font_eng_t *e, int finalized) {
    if (!e) return;
    pthread_mutex_lock(&font_eng_mu);
    if (!--e->inside) {
        if (e->waiters) pthread_cond_broadcast(&font_eng_cv);
        else if (finalized) e->self = NULL, e->owner = 0;   // Finalize: the slot is free for another engine (an engine made again at the same address takes it anew)
    }
    pthread_mutex_unlock(&font_eng_mu);
}
// hle_sdk16.c's take-overs of Initialize and LoadFont come here too, and hle_thread.c gives a blocking thread's engines back
static void *font_enter_load(void *self, const char *what) { return font_enter(self, what); }
static void font_leave_load(void *e) { font_leave(e, 0); }
__attribute__((constructor)) static void font_attach(void) {
    pthread_condattr_t a; pthread_condattr_init(&a);
#ifdef __ANDROID__
    pthread_condattr_setclock(&a, FONT_CLOCK);
#endif
    pthread_cond_init(&font_eng_cv, &a); pthread_condattr_destroy(&a);
    dm_fontll_enter = font_enter_load; dm_fontll_leave = font_leave_load; dm_core_excl_release = font_release_self;
}

typedef uint64_t (*font_fn_t)(void *, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t);
static font_fn_t font_orig(font_fn_t *cache, const char *mangled) {
    font_fn_t f = __atomic_load_n(cache, __ATOMIC_ACQUIRE);
    if (!f) {
        if (!(f = (font_fn_t)guest_sym(mangled))) die("%s not found in the guest", mangled);
        __atomic_store_n(cache, f, __ATOMIC_RELEASE);
    }
    return f;
}
// Every member of the engine that works on its font state; the arguments are integers and pointers, the result is in x0 (or none), so all of them pass as eight
//   integer registers. Not taken over: the constructor and destructor, GetPointerToWorkBuffer (hle_sdk16.c; it reads one field), FinalizeOtfKerningTable and
//   BinarySearchForOtfKerning (they touch only the table), PickKerningFromGposTable (static)
#define FONT_CALL(mangled, cname, fin) \
    HLE_FORCE(mangled, "one thread at a time in a font engine, as a Horizon core keeps it (hle_fontll.c); the engine itself stays in the SDK", uint64_t, cname, \
              void *self, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4, uint64_t a5, uint64_t a6, uint64_t a7) { \
        static font_fn_t orig; font_fn_t f = font_orig(&orig, mangled); \
        font_eng_t *e = font_enter(self, &#cname[9]);   /* the member's name, past "h_fontll_" */ \
        uint64_t r = f(self, a1, a2, a3, a4, a5, a6, a7); \
        font_leave(e, fin); \
        return r; \
    }
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine8FinalizeEv", h_fontll_Finalize, 1)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine14GetFontMetricsEPNS0_7MetricsE", h_fontll_GetFontMetrics, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine7SetFontEPc", h_fontll_SetFont, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine8SetScaleEiiii", h_fontll_SetScale, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine13SetBoldWeightEi", h_fontll_SetBoldWeight, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine15SetOutlineWidthEt", h_fontll_SetOutlineWidth, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine8SetFlagsENS1_5FlagsE", h_fontll_SetFlags, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine11SetAutoHintEb", h_fontll_SetAutoHint, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine10GetAdvanceEPsS2_PiS3_jt", h_fontll_GetAdvance, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine10GetKerningEPiS2_jj", h_fontll_GetKerning, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine22CheckTtfKernTableExistEv", h_fontll_CheckTtfKernTableExist, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine15CheckGlyphExistEj", h_fontll_CheckGlyphExist, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine15AcquireGlyphmapEjt", h_fontll_AcquireGlyphmap, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine13ReleasesGlyphEPv", h_fontll_ReleasesGlyph, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine8GetErrorEv", h_fontll_GetError, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine25InitializeOtfKerningTableEPFPvmmS2_ES2_", h_fontll_InitKerning, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine25InitializeOtfKerningTableEPFPvmmS2_ES2_b", h_fontll_InitKerningB, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine17AcquireOtfKerningEPKNS0_15OtfKerningTableEjjj", h_fontll_AcquireOtfKerning, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine22AcquireOtfKerningFirstEPKNS0_15OtfKerningTableEjj", h_fontll_AcquireOtfKerningFirst, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine21AcquireOtfKerningLastEPKNS0_15OtfKerningTableEjj", h_fontll_AcquireOtfKerningLast, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine20AcquireRawOtfKerningEPKNS0_15OtfKerningTableEjj", h_fontll_AcquireRawOtfKerning, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine25AcquireRawOtfKerningFirstEPKNS0_15OtfKerningTableEj", h_fontll_AcquireRawOtfKerningFirst, 0)
FONT_CALL("_ZN2nn6fontll18ScalableFontEngine24AcquireRawOtfKerningLastEPKNS0_15OtfKerningTableEj", h_fontll_AcquireRawOtfKerningLast, 0)
// PEANUTS end
