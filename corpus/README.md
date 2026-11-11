# Corpus

Each subdirectory of `pairs/` is one old/new firmware pair:

```text
pairs/<name>/old.srec
pairs/<name>/new.srec
pairs/<name>/README.md
```

`crab-cage delta sync-cases` writes `nuggets/delta-kernel/cases/<name>.in` from every pair, in directory-name order. A new pair is a new directory. Nothing else has to be registered.

`01-bugfix` through `04-insert` are compiled with `corpus/firmware/build.sh` (arm-none-eabi-gcc, freestanding, no newlib and no libgcc). `05-shift-inside` through `08-unaligned` are written by `crab-cage delta gen-synthetic`. The SREC files are committed, so the tests do not need the ARM toolchain.

The pair README says how that pair was produced.
