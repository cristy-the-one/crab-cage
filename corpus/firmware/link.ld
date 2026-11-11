/* Cortex-M image at the STM32 flash base. No RAM load region: there is no .data. */
ENTRY(reset_handler)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 64K
    RAM (xrw) : ORIGIN = 0x20000000, LENGTH = 16K
}

SECTIONS
{
    .text : {
        KEEP(*(.isr_vector))
        *(.text.reset)
        *(.text.extra)
        *(.text.worker)
        *(.text*)
        *(.rodata*)
    } > FLASH

    .bss : {
        *(.bss*)
    } > RAM

    /DISCARD/ : {
        *(.ARM.exidx*)
        *(.ARM.extab*)
        *(.comment)
        *(.eh_frame*)
    }
}
