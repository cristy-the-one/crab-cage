/* EXTRA=0 is the old image. EXTRA=1 inserts extra() between reset and worker,
   which shifts worker to a higher address inside the same sector. */
typedef unsigned int u32;

void reset_handler(void);
int worker(int x);

__attribute__((section(".isr_vector"), used))
const u32 vectors[2] = {
    0x20001000u,
    (u32)reset_handler,
};

#if EXTRA
__attribute__((noinline, used, section(".text.extra")))
int extra(int x) {
    return (x * 5) + 1;
}
#endif

__attribute__((noinline, used, section(".text.worker")))
int worker(int x) {
    /* Register-only, so the bytes stay identical when the function moves. */
    x = (x * 3) + 7;
    x = (x * 3) + 11;
    x = (x * 3) + 13;
    x = (x * 3) + 17;
    x = (x * 3) + 19;
    x = (x * 3) + 23;
    return x;
}

__attribute__((used, section(".text.reset")))
void reset_handler(void) {
    volatile int n = worker(4);
#if EXTRA
    n += extra(n);
#endif
    (void)n;
    for (;;) {
    }
}
