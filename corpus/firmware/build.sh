#!/bin/sh
# Rebuild the compiled corpus pairs. CI does not run this: the SREC files
# under corpus/pairs are the inputs the tests use. No library is linked
# (no newlib, no libgcc).
set -eu
cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
CC="${CC:-arm-none-eabi-gcc}"
OBJCOPY="${OBJCOPY:-arm-none-eabi-objcopy}"
OBJDUMP="${OBJDUMP:-arm-none-eabi-objdump}"
CFLAGS="-mcpu=cortex-m4 -mthumb -Os -ffreestanding -nostdlib -nostartfiles -fno-builtin -fno-exceptions -fno-unwind-tables -fno-asynchronous-unwind-tables -Wall -Wextra"
GCC_VER="$("$CC" -dumpversion)"
GCC_ID="$("$CC" --version | head -n 1)"

build_one() {
    pair="$1"
    src="$2"
    ld="$3"
    old_defs="$4"
    new_defs="$5"
    about="$6"
    dest="$ROOT/corpus/pairs/$pair"
    mkdir -p "$dest"
    "$CC" $CFLAGS $old_defs -T "$ld" -o "$pair-old.elf" "$src"
    "$CC" $CFLAGS $new_defs -T "$ld" -o "$pair-new.elf" "$src"
    "$OBJCOPY" -O srec "$pair-old.elf" "$pair-old.srec"
    "$OBJCOPY" -O srec "$pair-new.elf" "$pair-new.srec"
    mv "$pair-old.srec" "$dest/old.srec"
    mv "$pair-new.srec" "$dest/new.srec"
    {
        printf '%s\n\n' "$about"
        printf 'Toolchain: %s\n' "$GCC_ID"
        printf 'Compiler version token: %s\n' "$GCC_VER"
        printf 'Rebuilt by corpus/firmware/build.sh. The SREC files are committed so CI does not need the ARM toolchain.\n\n'
        printf 'old section headers:\n'
        "$OBJDUMP" -h "$pair-old.elf"
        printf '\nnew section headers:\n'
        "$OBJDUMP" -h "$pair-new.elf"
        printf '\nold disassembly:\n'
        "$OBJDUMP" -d "$pair-old.elf"
        printf '\nnew disassembly:\n'
        "$OBJDUMP" -d "$pair-new.elf"
    } > "$dest/README.md"
    rm -f "$pair-old.elf" "$pair-new.elf"
}

build_one 01-bugfix bugfix.c link.ld "-DSEED=1" "-DSEED=2" \
"Compiled bugfix pair. bugfix.c worker() returns (x * 3) + SEED. The old image uses -DSEED=1 and the new image uses -DSEED=2, which is a same-width Thumb immediate. Linked with link.ld at 0x08000000."

build_one 02-table table.c link.ld "-DVER=0" "-DVER=0xFF" \
"Compiled table pair. table.c holds a 256-byte cal[] table. CELL(i) is ((i * 17) ^ 0x5A), and every index divisible by 17 is xored with VER. The old image uses -DVER=0 and the new image uses -DVER=0xFF. The instructions are the same in both images."

build_one 03-gaps gaps.c gaps.ld "-DSEED=1" "-DSEED=2" \
"Compiled gap pair. gaps.ld places .rodata at the next 2 KB boundary after .text, so the SREC has a hole and a second sector. SEED changes one immediate in worker(), from 1 to 2. marker[] is identical, so the second sector does not need a STORE."

build_one 04-insert insert.c link.ld "-DEXTRA=0" "-DEXTRA=1" \
"Compiled insert pair. insert.c with -DEXTRA=0 is the old image. -DEXTRA=1 inserts extra() between reset_handler and worker(), so worker() moves to a higher address in the same sector. worker() is a chain of register-only multiply-adds, so its bytes do not change when it moves."
