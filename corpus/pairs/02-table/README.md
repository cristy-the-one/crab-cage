Compiled table pair. table.c holds a 256-byte cal[] table. CELL(i) is ((i * 17) ^ 0x5A), and every index divisible by 17 is xored with VER. The old image uses -DVER=0 and the new image uses -DVER=0xFF. The instructions are the same in both images.

Toolchain: arm-none-eabi-gcc (15:13.2.rel1-2) 13.2.1 20231009
Compiler version token: 13.2.1
Rebuilt by corpus/firmware/build.sh. The SREC files are committed so CI does not need the ARM toolchain.

old section headers:

02-table-old.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000124  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  00001124  2**0
                  CONTENTS, READONLY

new section headers:

02-table-new.elf:     file format elf32-littlearm

Sections:
Idx Name          Size      VMA       LMA       File off  Algn
  0 .text         00000124  08000000  08000000  00001000  2**2
                  CONTENTS, ALLOC, LOAD, READONLY, CODE
  1 .ARM.attributes 0000002e  00000000  00000000  00001124  2**0
                  CONTENTS, READONLY

old disassembly:

02-table-old.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f804 	bl	8000018 <worker>
 8000010:	9001      	str	r0, [sp, #4]
 8000012:	9b01      	ldr	r3, [sp, #4]
 8000014:	e7fe      	b.n	8000014 <reset_handler+0xc>
	...

08000018 <worker>:
 8000018:	4b01      	ldr	r3, [pc, #4]	@ (8000020 <worker+0x8>)
 800001a:	b2c0      	uxtb	r0, r0
 800001c:	5c18      	ldrb	r0, [r3, r0]
 800001e:	4770      	bx	lr
 8000020:	08000024 	.word	0x08000024

08000024 <cal>:
 8000024:	69784b5a 2d3c0f1e e1f0c3d2 a5b48796     ZKxi..<-........
 8000034:	19687b4a dd2c3f0e 91e0f3c2 55a4b786     J{h..?,........U
 8000044:	09186b7a cddc2f3e 8190e3f2 4554a7b6     zk..>/........TE
 8000054:	39081b6a fdccdf2e b18093e2 754457a6     j..9.........WDu
 8000064:	29380b1a edfccfde a1b08392 65744756     ..8)........VGte
 8000074:	d9283b0a 9decffce 51a0b382 15647746     .;(........QFwd.
 8000084:	c9d82b3a 8d9ceffe 4150a3b2 05146776     :+........PAvg..
 8000094:	f9c8db2a bd8c9fee 714053a2 35041766     *........S@qf..5
 80000a4:	e9f8cbda adbc8f9e 61704352 25340716     ........RCpa..4%
 80000b4:	99e8fbca 5dacbf8e 11607342 d5243706     .......]Bs`..7$.
 80000c4:	8998ebfa 4d5cafbe 01106372 c5d42736     ......\Mrc..6'..
 80000d4:	b9889bea 7d4c5fae 31001362 f5c4d726     ....._L}b..1&...
 80000e4:	a9b88b9a 6d7c4f5e 21300312 e5f4c7d6     ....^O|m..0!....
 80000f4:	59a8bb8a 1d6c7f4e d1203302 95e4f7c6     ...YN.l..3 .....
 8000104:	4958abba 0d1c6f7e c1d02332 8594e7f6     ..XI~o..2#......
 8000114:	79485baa 3d0c1f6e f1c0d322 b58497e6     .[Hyn..=".......

new disassembly:

02-table-new.elf:     file format elf32-littlearm


Disassembly of section .text:

08000000 <vectors>:
 8000000:	00 10 00 20 09 00 00 08                             ... ....

08000008 <reset_handler>:
 8000008:	b507      	push	{r0, r1, r2, lr}
 800000a:	2004      	movs	r0, #4
 800000c:	f000 f804 	bl	8000018 <worker>
 8000010:	9001      	str	r0, [sp, #4]
 8000012:	9b01      	ldr	r3, [sp, #4]
 8000014:	e7fe      	b.n	8000014 <reset_handler+0xc>
	...

08000018 <worker>:
 8000018:	4b01      	ldr	r3, [pc, #4]	@ (8000020 <worker+0x8>)
 800001a:	b2c0      	uxtb	r0, r0
 800001c:	5c18      	ldrb	r0, [r3, r0]
 800001e:	4770      	bx	lr
 8000020:	08000024 	.word	0x08000024

08000024 <cal>:
 8000024:	69784ba5 2d3c0f1e e1f0c3d2 a5b48796     .Kxi..<-........
 8000034:	1968844a dd2c3f0e 91e0f3c2 55a4b786     J.h..?,........U
 8000044:	09e76b7a cddc2f3e 8190e3f2 4554a7b6     zk..>/........TE
 8000054:	c6081b6a fdccdf2e b18093e2 754457a6     j............WDu
 8000064:	29380b1a edfccf21 a1b08392 65744756     ..8)!.......VGte
 8000074:	d9283b0a 9dec00ce 51a0b382 15647746     .;(........QFwd.
 8000084:	c9d82b3a 8d63effe 4150a3b2 05146776     :+....c...PAvg..
 8000094:	f9c8db2a 428c9fee 714053a2 35041766     *......B.S@qf..5
 80000a4:	e9f8cbda adbc8f9e 617043ad 25340716     .........Cpa..4%
 80000b4:	99e8fbca 5dacbf8e 11608c42 d5243706     .......]B.`..7$.
 80000c4:	8998ebfa 4d5cafbe 01ef6372 c5d42736     ......\Mrc..6'..
 80000d4:	b9889bea 7d4c5fae ce001362 f5c4d726     ....._L}b...&...
 80000e4:	a9b88b9a 6d7c4f5e 21300312 e5f4c729     ....^O|m..0!)...
 80000f4:	59a8bb8a 1d6c7f4e d1203302 95e408c6     ...YN.l..3 .....
 8000104:	4958abba 0d1c6f7e c1d02332 856be7f6     ..XI~o..2#....k.
 8000114:	79485baa 3d0c1f6e f1c0d322 4a8497e6     .[Hyn..="......J
