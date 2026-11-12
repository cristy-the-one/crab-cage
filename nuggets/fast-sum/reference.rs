    // n (n + 1) (2n + 1) / 6. Divide out the 2 and the 3 first, so every
    // factor is reduced before the products are taken modulo MODULUS.
    let mut a = n;
    let mut b = n + 1;
    let mut c = 2 * n + 1;
    if a % 2 == 0 {
        a /= 2;
    } else {
        b /= 2;
    }
    if a % 3 == 0 {
        a /= 3;
    } else if b % 3 == 0 {
        b /= 3;
    } else {
        c /= 3;
    }
    (a % MODULUS) * (b % MODULUS) % MODULUS * (c % MODULUS) % MODULUS
