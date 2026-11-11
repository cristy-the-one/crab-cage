Compiled insert pair. insert.c with -DEXTRA=0 is the old image. -DEXTRA=1 inserts extra() between reset_handler and worker(), so worker() moves to a higher address in the same sector. worker() is a chain of register-only multiply-adds, so its bytes do not change when it moves.

Toolchain: arm-none-eabi-gcc (15:13.2.rel1-2) 13.2.1 20231009
Compiler version token: 13.2.1
Rebuilt by corpus/firmware/build.sh. The SREC files are committed so CI does not need the ARM toolchain.

old section headers:

04-insert-old.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000034  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  00001034  2**0
                  CONTENTS, READONLY

new section headers:

04-insert-new.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000048  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  00001048  2**0
                  CONTENTS, READONLY

old disassembly:

04-insert-old.elf:     file format elf32-littlearm


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
 800001a:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800001e:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 8000022:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 8000026:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800002a:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800002e:	f600 4068 	addw	r0, r0, #3176	@ 0xc68
 8000032:	4770      	bx	lr

new disassembly:

04-insert-new.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f80d 	bl	800002a <worker>
 8000010:	9001      	str	r0, [sp, #4]
 8000012:	9801      	ldr	r0, [sp, #4]
 8000014:	f000 f805 	bl	8000022 <extra>
 8000018:	9b01      	ldr	r3, [sp, #4]
 800001a:	4403      	add	r3, r0
 800001c:	9301      	str	r3, [sp, #4]
 800001e:	9b01      	ldr	r3, [sp, #4]
 8000020:	e7fe      	b.n	8000020 <reset_handler+0x18>

08000022 <extra>:
 8000022:	eb00 0080 	add.w	r0, r0, r0, lsl #2
 8000026:	3001      	adds	r0, #1
 8000028:	4770      	bx	lr

0800002a <worker>:
 800002a:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800002e:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 8000032:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 8000036:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800003a:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 800003e:	eb00 0040 	add.w	r0, r0, r0, lsl #1
 8000042:	f600 4068 	addw	r0, r0, #3176	@ 0xc68
 8000046:	4770      	bx	lr
