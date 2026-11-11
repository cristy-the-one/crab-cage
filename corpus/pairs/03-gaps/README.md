Compiled gap pair. gaps.ld places .rodata at the next 2 KB boundary after .text, so the SREC has a hole and a second sector. SEED changes one immediate in worker(), from 1 to 2. marker[] is identical, so the second sector does not need a STORE.

Toolchain: arm-none-eabi-gcc (15:13.2.rel1-2) 13.2.1 20231009
Compiler version token: 13.2.1
Rebuilt by corpus/firmware/build.sh. The SREC files are committed so CI does not need the ARM toolchain.

old section headers:

03-gaps-old.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000020  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .rodata       00000020  08000800  08000800  00001800  2**0
                  CONTENTS, ALLOC, LOAD, READONLY, DATA
  2 .ARM.attributes 0000002e  00000000  00000000  00001820  2**0
                  CONTENTS, READONLY

new section headers:

03-gaps-new.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000020  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .rodata       00000020  08000800  08000800  00001800  2**0
                  CONTENTS, ALLOC, LOAD, READONLY, DATA
  2 .ARM.attributes 0000002e  00000000  00000000  00001820  2**0
                  CONTENTS, READONLY

old disassembly:

03-gaps-old.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f804 	bl	8000018 <worker>
 8000010:	30a5      	adds	r0, #165	@ 0xa5
 8000012:	9001      	str	r0, [sp, #4]
 8000014:	9b01      	ldr	r3, [sp, #4]
 8000016:	e7fe      	b.n	8000016 <reset_handler+0xe>

08000018 <worker>:
 8000018:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800001c:	3001      	adds	r0, #1
 800001e:	4770      	bx	lr

new disassembly:

03-gaps-new.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f804 	bl	8000018 <worker>
 8000010:	30a5      	adds	r0, #165	@ 0xa5
 8000012:	9001      	str	r0, [sp, #4]
 8000014:	9b01      	ldr	r3, [sp, #4]
 8000016:	e7fe      	b.n	8000016 <reset_handler+0xe>

08000018 <worker>:
 8000018:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800001c:	3002      	adds	r0, #2
 800001e:	4770      	bx	lr
