/* One Thumb immediate changes width-for-width. SEED is 1 in the old image and 2 in the new. */
typedef unsigned int u32;

void reset_handler(void);
int worker(int x);

__attribute__((section(".isr_vector"), used))
const u32 vectors[2] = {
    0x20001000u,
    (u32)reset_handler,
};

__attribute__((noinline, used, section(".text.worker")))
int worker(int x) {
    return (x * 3) + SEED;
}

__attribute__((used, section(".text.reset")))
void reset_handler(void) {
    volatile int n = worker(4);
    (void)n;
    for (;;) {
    }
}
