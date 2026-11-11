# Compiled images

`build.sh` builds four Cortex-M4 images with `arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -Os -ffreestanding -nostdlib -nostartfiles`. Nothing from newlib or libgcc is linked. `objcopy -O srec` writes the pairs under `corpus/pairs/`.

| Pair | Source | Old flags | New flags | What changed |
|---|---|---|---|---|
| `01-bugfix` | `bugfix.c` + `link.ld` | `-DSEED=1` | `-DSEED=2` | one Thumb immediate |
| `02-table` | `table.c` + `link.ld` | `-DVER=0` | `-DVER=0xFF` | every 17th byte of a 256-byte table |
| `03-gaps` | `gaps.c` + `gaps.ld` | `-DSEED=1` | `-DSEED=2` | one immediate, and `.rodata` at `0x08000800` so the SREC has a hole |
| `04-insert` | `insert.c` + `link.ld` | `-DEXTRA=0` | `-DEXTRA=1` | `extra()` inserted in front of `worker()`, which shifts `worker()` |

Flash starts at `0x08000000`. Each pair directory's README has the compiler identity and the disassembly from the build that produced the committed SREC files.
