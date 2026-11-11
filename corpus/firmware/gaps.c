/* Text, then a linker gap to the next 2 KB, then 32 bytes of .rodata.
   SEED changes one instruction immediate. The table is identical in both images,
   so the second sector is not part of the delta. */
typedef unsigned int u32;
typedef unsigned char u8;

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

__attribute__((used, section(".rodata")))
const u8 marker[32] = {
    0xA5, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
    0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
};

__attribute__((used, section(".text.reset")))
void reset_handler(void) {
    volatile int n = worker(4) + (int)marker[0];
    (void)n;
    for (;;) {
    }
}
