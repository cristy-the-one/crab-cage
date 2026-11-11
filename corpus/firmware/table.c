/* 256-byte calibration table. VER is 0 in the old image and 0xFF in the new.
   Every 17th entry is xored with VER. The instructions do not change. */
typedef unsigned int u32;
typedef unsigned char u8;

void reset_handler(void);
int worker(int x);

#define CELL(i) ((u8)((((i) * 17) ^ 0x5A) ^ ((((i) % 17) == 0) ? (VER) : 0)))

__attribute__((section(".isr_vector"), used))
const u32 vectors[2] = {
    0x20001000u,
    (u32)reset_handler,
};

__attribute__((used, section(".rodata")))
const u8 cal[256] = {
    CELL(0), CELL(1), CELL(2), CELL(3), CELL(4), CELL(5), CELL(6), CELL(7),
    CELL(8), CELL(9), CELL(10), CELL(11), CELL(12), CELL(13), CELL(14), CELL(15),
    CELL(16), CELL(17), CELL(18), CELL(19), CELL(20), CELL(21), CELL(22), CELL(23),
    CELL(24), CELL(25), CELL(26), CELL(27), CELL(28), CELL(29), CELL(30), CELL(31),
    CELL(32), CELL(33), CELL(34), CELL(35), CELL(36), CELL(37), CELL(38), CELL(39),
    CELL(40), CELL(41), CELL(42), CELL(43), CELL(44), CELL(45), CELL(46), CELL(47),
    CELL(48), CELL(49), CELL(50), CELL(51), CELL(52), CELL(53), CELL(54), CELL(55),
    CELL(56), CELL(57), CELL(58), CELL(59), CELL(60), CELL(61), CELL(62), CELL(63),
    CELL(64), CELL(65), CELL(66), CELL(67), CELL(68), CELL(69), CELL(70), CELL(71),
    CELL(72), CELL(73), CELL(74), CELL(75), CELL(76), CELL(77), CELL(78), CELL(79),
    CELL(80), CELL(81), CELL(82), CELL(83), CELL(84), CELL(85), CELL(86), CELL(87),
    CELL(88), CELL(89), CELL(90), CELL(91), CELL(92), CELL(93), CELL(94), CELL(95),
    CELL(96), CELL(97), CELL(98), CELL(99), CELL(100), CELL(101), CELL(102), CELL(103),
    CELL(104), CELL(105), CELL(106), CELL(107), CELL(108), CELL(109), CELL(110), CELL(111),
    CELL(112), CELL(113), CELL(114), CELL(115), CELL(116), CELL(117), CELL(118), CELL(119),
    CELL(120), CELL(121), CELL(122), CELL(123), CELL(124), CELL(125), CELL(126), CELL(127),
    CELL(128), CELL(129), CELL(130), CELL(131), CELL(132), CELL(133), CELL(134), CELL(135),
    CELL(136), CELL(137), CELL(138), CELL(139), CELL(140), CELL(141), CELL(142), CELL(143),
    CELL(144), CELL(145), CELL(146), CELL(147), CELL(148), CELL(149), CELL(150), CELL(151),
    CELL(152), CELL(153), CELL(154), CELL(155), CELL(156), CELL(157), CELL(158), CELL(159),
    CELL(160), CELL(161), CELL(162), CELL(163), CELL(164), CELL(165), CELL(166), CELL(167),
    CELL(168), CELL(169), CELL(170), CELL(171), CELL(172), CELL(173), CELL(174), CELL(175),
    CELL(176), CELL(177), CELL(178), CELL(179), CELL(180), CELL(181), CELL(182), CELL(183),
    CELL(184), CELL(185), CELL(186), CELL(187), CELL(188), CELL(189), CELL(190), CELL(191),
    CELL(192), CELL(193), CELL(194), CELL(195), CELL(196), CELL(197), CELL(198), CELL(199),
    CELL(200), CELL(201), CELL(202), CELL(203), CELL(204), CELL(205), CELL(206), CELL(207),
    CELL(208), CELL(209), CELL(210), CELL(211), CELL(212), CELL(213), CELL(214), CELL(215),
    CELL(216), CELL(217), CELL(218), CELL(219), CELL(220), CELL(221), CELL(222), CELL(223),
    CELL(224), CELL(225), CELL(226), CELL(227), CELL(228), CELL(229), CELL(230), CELL(231),
    CELL(232), CELL(233), CELL(234), CELL(235), CELL(236), CELL(237), CELL(238), CELL(239),
    CELL(240), CELL(241), CELL(242), CELL(243), CELL(244), CELL(245), CELL(246), CELL(247),
    CELL(248), CELL(249), CELL(250), CELL(251), CELL(252), CELL(253), CELL(254), CELL(255),
};

__attribute__((noinline, used, section(".text.worker")))
int worker(int x) {
    return (int)cal[x & 255];
}

__attribute__((used, section(".text.reset")))
void reset_handler(void) {
    volatile int n = worker(4);
    (void)n;
    for (;;) {
    }
}
