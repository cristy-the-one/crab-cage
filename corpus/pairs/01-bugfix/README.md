Compiled bugfix pair. bugfix.c worker() returns (x * 3) + SEED. The old image uses -DSEED=1 and the new image uses -DSEED=2, which is a same-width Thumb immediate. Linked with link.ld at 0x08000000.

Toolchain: arm-none-eabi-gcc (15:13.2.rel1-2) 13.2.1 20231009
Compiler version token: 13.2.1
Rebuilt by corpus/firmware/build.sh. The SREC files are committed so CI does not need the ARM toolchain.

old section headers:

01-bugfix-old.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         0000001e  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  0000101e  2**0
                  CONTENTS, READONLY

new section headers:

01-bugfix-new.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         0000001e  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  0000101e  2**0
                  CONTENTS, READONLY

old disassembly:

01-bugfix-old.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f803 	bl	8000016 <worker>
 8000010:	9001      	str	r0, [sp, #4]
 8000012:	9b01      	ldr	r3, [sp, #4]
 8000014:	e7fe      	b.n	8000014 <reset_handler+0xc>

08000016 <worker>:
 8000016:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800001a:	3001      	adds	r0, #1
 800001c:	4770      	bx	lr

new disassembly:

01-bugfix-new.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f803 	bl	8000016 <worker>
 8000010:	9001      	str	r0, [sp, #4]
 8000012:	9b01      	ldr	r3, [sp, #4]
 8000014:	e7fe      	b.n	8000014 <reset_handler+0xc>

08000016 <worker>:
 8000016:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800001a:	3002      	adds	r0, #2
 800001c:	4770      	bx	lr
