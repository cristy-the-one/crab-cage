/* Same image as link.ld, with .rodata forced to the next 2 KB boundary. */
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
        *(.text.worker)
        *(.text*)
    } > FLASH

    /* A second load region, one sector later. The bytes between the two
       sections are not in any section, so the SREC has a hole. */
    .rodata 0x08000800 : {
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
