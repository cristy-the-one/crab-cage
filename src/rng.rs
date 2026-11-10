/// Portable PCG-XSH-RR. The GA and the plant noise use this so a seed
/// produces the same trajectory on Linux and Windows.

#[derive(Clone, Debug)]
pub struct Pcg64 {
    state: u64,
    inc: u64,
}

impl Pcg64 {
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Self {
            state: 0,
            inc: (stream << 1) | 1,
        };
        let _ = rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        let _ = rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6364136223846793005).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    pub fn next_f64(&mut self) -> f64 {
        let hi = self.next_u32() as u64;
        let lo = self.next_u32() as u64;
        let bits = (hi << 32) | lo;
        (bits >> 11) as f64 / ((1u64 << 53) as f64)
    }

    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Box-Muller. `next_f64` is in `[0, 1)`; the log argument is clamped.
    pub fn gauss(&mut self) -> f64 {
        let u1 = self.next_f64().clamp(1e-12, 1.0 - f64::EPSILON);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn gen_index(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        (self.next_f64() * n as f64) as usize % n
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Pcg64::new(7, 1);
        let mut b = Pcg64::new(7, 1);
        for _ in 0..32 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn gauss_is_finite() {
        let mut rng = Pcg64::new(1, 2);
        for _ in 0..100 {
            let g = rng.gauss();
            assert!(g.is_finite());
        }
    }
}
